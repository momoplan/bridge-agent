use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::ffi::CStr;
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{Duration, Instant};
#[cfg(unix)]
use uuid::Uuid;
#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
#[cfg(windows)]
use winreg::RegKey;

#[cfg(unix)]
const USER_SHELL_PATH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const USER_SHELL_PATH_PROBE_MAX_BYTES: u64 = 1024 * 1024;

/// Adds the current user's command search path to a child-process environment.
///
/// Connector manifests remain the source of connector-specific environment
/// entries. The host contributes dynamic device state only when the process is
/// launched, so a PATH discovered from the current user is never persisted in
/// agent-config.json.
pub fn enrich_user_command_environment(
    executable: Option<&str>,
    environment: &mut BTreeMap<String, String>,
) {
    let user_path = current_user_command_path();
    enrich_user_command_environment_with_path(executable, environment, user_path.as_deref());
}

/// Applies an already resolved user command path. This keeps one lifecycle
/// operation deterministic when its caller has resolved the user environment
/// at the operation boundary.
pub fn enrich_user_command_environment_with_path(
    executable: Option<&str>,
    environment: &mut BTreeMap<String, String>,
    user_path: Option<&str>,
) {
    let declared_path = environment.get("PATH").cloned();
    let host_path = env::var("PATH").ok();
    let merged = merge_command_paths(
        executable,
        declared_path.as_deref(),
        user_path,
        host_path.as_deref(),
    );
    if let Some(merged) = merged {
        environment.insert("PATH".to_string(), merged);
    }
}

/// Resolves the current desktop user's command path rather than trusting the
/// environment inherited by a GUI process.
pub fn current_user_command_path() -> Option<String> {
    #[cfg(unix)]
    {
        match unix_account_shell().and_then(|shell| {
            probe_user_shell_path(&shell, USER_SHELL_PATH_PROBE_TIMEOUT)
                .with_context(|| format!("probe user shell {}", shell.display()))
        }) {
            Ok(path) => Some(path),
            Err(error) => {
                tracing::warn!(error = %error, "failed to resolve current user command PATH");
                None
            }
        }
    }
    #[cfg(windows)]
    {
        match windows_user_command_path() {
            Ok(path) => Some(path),
            Err(error) => {
                tracing::warn!(error = %error, "failed to resolve current user command PATH");
                None
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn merge_command_paths(
    executable: Option<&str>,
    declared_path: Option<&str>,
    user_path: Option<&str>,
    host_path: Option<&str>,
) -> Option<String> {
    let mut entries = Vec::new();
    if let Some(parent) = executable
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
    {
        push_unique_path(&mut entries, parent.to_path_buf());
    }
    for path in [declared_path, user_path, host_path].into_iter().flatten() {
        for entry in env::split_paths(path) {
            push_unique_path(&mut entries, entry);
        }
    }
    if entries.is_empty() {
        return None;
    }
    env::join_paths(entries)
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

fn push_unique_path(entries: &mut Vec<PathBuf>, entry: PathBuf) {
    if entry.as_os_str().is_empty() || entries.iter().any(|candidate| candidate == &entry) {
        return;
    }
    entries.push(entry);
}

#[cfg(unix)]
fn unix_account_shell() -> Result<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let suggested_size = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut buffer_size = if suggested_size > 0 {
        usize::try_from(suggested_size).unwrap_or(16 * 1024)
    } else {
        16 * 1024
    }
    .max(1024);

    loop {
        let mut password = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0_u8; buffer_size];
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                password.as_mut_ptr(),
                buffer.as_mut_ptr().cast::<libc::c_char>(),
                buffer.len(),
                &mut result,
            )
        };
        if code == libc::ERANGE {
            buffer_size = buffer_size
                .checked_mul(2)
                .filter(|size| *size <= 1024 * 1024)
                .context("user account record exceeds the supported size")?;
            continue;
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code)).context("read current user account");
        }
        if result.is_null() {
            bail!("current user account was not found");
        }
        let shell = unsafe { CStr::from_ptr((*result).pw_shell) };
        if shell.to_bytes().is_empty() {
            bail!("current user account has no login shell");
        }
        let shell = PathBuf::from(std::ffi::OsString::from_vec(shell.to_bytes().to_vec()));
        if !shell.is_absolute() || !shell.is_file() {
            bail!(
                "current user login shell is not an executable file: {}",
                shell.display()
            );
        }
        return Ok(shell);
    }
}

#[cfg(unix)]
fn probe_user_shell_path(shell: &Path, timeout: Duration) -> Result<String> {
    let marker = format!("BRIDGE_AGENT_USER_PATH_{}", Uuid::new_v4().simple());
    let probe = format!("/usr/bin/printf '\\036{marker}\\037'; /usr/bin/env -0");
    let stdout_capture =
        tempfile::NamedTempFile::new().context("create current user shell PATH capture")?;
    let mut command = Command::new(shell);
    configure_shell_probe_command(&mut command, shell, &probe);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            stdout_capture
                .reopen()
                .context("open current user shell PATH capture")?,
        ))
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command
        .spawn()
        .with_context(|| format!("start current user shell {}", shell.display()))?;
    let pid = child.id();
    let deadline = Instant::now() + timeout;
    let status = loop {
        let output_size = stdout_capture
            .as_file()
            .metadata()
            .context("inspect current user shell PATH capture")?
            .len();
        if output_size > USER_SHELL_PATH_PROBE_MAX_BYTES {
            terminate_probe_process_group(pid, &mut child);
            bail!(
                "current user shell PATH probe exceeded {} bytes",
                USER_SHELL_PATH_PROBE_MAX_BYTES
            );
        }
        if let Some(status) = child.try_wait().context("wait for current user shell")? {
            break status;
        }
        if Instant::now() >= deadline {
            terminate_probe_process_group(pid, &mut child);
            bail!(
                "current user shell PATH probe timed out after {}ms",
                timeout.as_millis()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if !status.success() {
        bail!("current user shell PATH probe exited with {status}");
    }

    let output =
        std::fs::read(stdout_capture.path()).context("read current user shell PATH capture")?;
    let marker = [b"\x1e".as_slice(), marker.as_bytes(), b"\x1f".as_slice()].concat();
    let marker_offset = output
        .windows(marker.len())
        .rposition(|candidate| candidate == marker)
        .context("current user shell PATH probe marker is missing")?
        + marker.len();
    for field in output[marker_offset..].split(|byte| *byte == 0) {
        if let Some(path) = field.strip_prefix(b"PATH=") {
            if path.is_empty() {
                bail!("current user shell returned an empty PATH");
            }
            return String::from_utf8(path.to_vec())
                .context("current user shell PATH is not valid UTF-8");
        }
    }
    bail!("current user shell environment did not contain PATH")
}

#[cfg(unix)]
fn configure_shell_probe_command(command: &mut Command, shell: &Path, probe: &str) {
    let shell_name = shell.file_name().and_then(|name| name.to_str());
    if matches!(shell_name, Some("csh" | "tcsh")) {
        // csh treats a leading '-' in argv[0] as the login-shell contract and
        // accepts -i/-c together, while rejecting -l combined with any option.
        let argv0 = format!("-{}", shell_name.unwrap_or("csh"));
        command.arg0(argv0).args(["-ic", probe]);
    } else {
        command.args(["-ilc", probe]);
    }
}

#[cfg(unix)]
fn terminate_probe_process_group(pid: u32, child: &mut std::process::Child) {
    if let Ok(process_group) = libc::pid_t::try_from(pid) {
        unsafe {
            libc::killpg(process_group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(windows)]
fn windows_user_command_path() -> Result<String> {
    let mut entries = Vec::new();
    for (hive, key) in [
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
        (HKEY_CURRENT_USER, r"Environment"),
    ] {
        if let Some(path) = windows_registry_path_value(hive, key) {
            for entry in env::split_paths(&path) {
                push_unique_path(&mut entries, entry);
            }
        }
    }
    if entries.is_empty() {
        bail!("Windows user and machine environment do not contain PATH");
    }
    env::join_paths(entries)
        .context("join Windows user command PATH")
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn windows_registry_path_value(hive: winreg::HKEY, key: &str) -> Option<String> {
    let key = RegKey::predef(hive).open_subkey(key).ok()?;
    key.get_value::<String, _>("Path")
        .ok()
        .map(|path| expand_windows_environment(&path))
}

#[cfg(windows)]
fn expand_windows_environment(value: &str) -> String {
    let mut expanded = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('%') else {
            expanded.push('%');
            expanded.push_str(after_start);
            return expanded;
        };
        let key = &after_start[..end];
        match env::var(key) {
            Ok(environment_value) => expanded.push_str(&environment_value),
            Err(_) => {
                expanded.push('%');
                expanded.push_str(key);
                expanded.push('%');
            }
        }
        rest = &after_start[end + 1..];
    }
    expanded.push_str(rest);
    expanded
}

#[cfg(test)]
mod tests {
    use super::merge_command_paths;
    use std::env;
    use std::path::PathBuf;

    #[test]
    fn command_path_merge_has_stable_precedence_and_deduplicates() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let executable = if cfg!(windows) {
            r"C:\\connector\\bin\\worker.exe"
        } else {
            "/connector/bin/worker"
        };
        let declared = ["/declared/bin", "/shared/bin"].join(separator);
        let user = ["/user/bin", "/shared/bin"].join(separator);
        let host = ["/host/bin", "/user/bin"].join(separator);

        let merged =
            merge_command_paths(Some(executable), Some(&declared), Some(&user), Some(&host))
                .unwrap();
        let entries = env::split_paths(&merged).collect::<Vec<_>>();
        let expected_parent = PathBuf::from(executable).parent().unwrap().to_path_buf();
        assert_eq!(
            entries,
            vec![
                expected_parent,
                PathBuf::from("/declared/bin"),
                PathBuf::from("/shared/bin"),
                PathBuf::from("/user/bin"),
                PathBuf::from("/host/bin"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn interactive_shell_probe_ignores_startup_output() {
        use super::probe_user_shell_path;
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;
        use std::time::Duration;

        let temporary = tempfile::tempdir().unwrap();
        let expected = temporary.path().join("user-bin");
        fs::create_dir(&expected).unwrap();
        let shell = temporary.path().join("test-shell");
        fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf 'startup output\\nPATH=wrong\\0'\nPATH='{}'\nexport PATH\nexec /bin/sh -c \"$2\"\n",
                expected.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&shell).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell, permissions).unwrap();

        let path = probe_user_shell_path(&shell, Duration::from_secs(2)).unwrap();
        assert_eq!(path, expected.display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn current_account_resolves_an_executable_shell() {
        use super::unix_account_shell;

        let shell = unix_account_shell().expect("current user login shell");
        assert!(shell.is_absolute());
        assert!(shell.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn interactive_shell_probe_is_bounded() {
        use super::probe_user_shell_path;
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;
        use std::time::{Duration, Instant};

        let temporary = tempfile::tempdir().unwrap();
        let shell = temporary.path().join("slow-shell");
        fs::write(&shell, "#!/bin/sh\nsleep 10\n").unwrap();
        let mut permissions = fs::metadata(&shell).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shell, permissions).unwrap();

        let started = Instant::now();
        let error = probe_user_shell_path(&shell, Duration::from_millis(75)).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn c_shell_probe_uses_its_login_argv_contract() {
        use super::probe_user_shell_path;
        use std::path::Path;
        use std::time::Duration;

        let shell = Path::new("/bin/csh");
        if !shell.is_file() {
            return;
        }
        let path = probe_user_shell_path(shell, Duration::from_secs(2)).unwrap();
        assert!(env::split_paths(&path).next().is_some());
    }
}
