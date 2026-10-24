/// Caller-supplied environment keys that can redirect the executable, inject
/// code into it, or change interpreter startup. Host passthrough values for
/// identity variables such as `HOME` stay in place; callers cannot replace them.
fn caller_env_key_allowed(key: &str) -> bool {
    is_ascii_env_name(key) && !caller_env_key_is_dangerous(key)
}

fn is_ascii_env_name(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn caller_env_key_is_dangerous(key: &str) -> bool {
    const EXACT: &[&str] = &[
        "PATH",
        "PATHEXT",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "SHELL",
        "COMSPEC",
        "SYSTEMROOT",
        "WINDIR",
        "BASH_ENV",
        "ENV",
        "SHELLOPTS",
        "BASHOPTS",
        "ZDOTDIR",
        "INPUTRC",
        "PROMPT_COMMAND",
        "PS4",
        "IFS",
        "CDPATH",
        "GCONV_PATH",
        "GLIBC_TUNABLES",
        "MALLOC_CHECK_",
        "HOSTALIASES",
        "LOCALDOMAIN",
        "RES_OPTIONS",
        "NLSPATH",
        "LOCPATH",
        "SSLKEYLOGFILE",
        "ELECTRON_RUN_AS_NODE",
        "PERL5OPT",
        "PERL5LIB",
        "PERLLIB",
        "PERL5DB",
        "RUBYOPT",
        "RUBYLIB",
        "GEM_HOME",
        "GEM_PATH",
        "NODE_PATH",
    ];
    if EXACT.contains(&key) || key.ends_with("_OPTIONS") {
        return true;
    }
    const PREFIXES: &[&str] = &[
        "LD_", "DYLD_", "PYTHON", "PERL", "RUBY", "GIT_", "BASH_", "ZSH_",
    ];
    PREFIXES.iter().any(|prefix| key.starts_with(prefix))
}

fn is_bare_command_name(command: &str) -> bool {
    let path = Path::new(command);
    !command.contains('/')
        && !command.contains('\\')
        && !command.contains('\0')
        && path.components().count() == 1
        && matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
}

/// Resolve a bare command name against an absolute, trusted `PATH`.
/// Relative directories are ignored so a `PATH` entry of `.` cannot select a
/// binary from the working directory.
fn resolve_executable(name: &str, path: &str) -> Option<PathBuf> {
    if !is_bare_command_name(name) {
        return None;
    }
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        if let Some(resolved) = executable_in_dir(&dir, name) {
            return Some(resolved);
        }
    }
    None
}

fn executable_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if is_executable_file(&direct) {
        return Some(direct);
    }
    #[cfg(windows)]
    {
        if Path::new(name).extension().is_some() {
            return None;
        }
        for ext in windows_path_extensions() {
            let candidate = dir.join(format!("{name}{ext}"));
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return metadata.permissions().mode() & 0o111 != 0;
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(windows)]
fn windows_path_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| {
            let ext = if ext.starts_with('.') {
                ext.to_string()
            } else {
                format!(".{ext}")
            };
            ext.to_ascii_uppercase()
        })
        .collect()
}

fn trusted_shell_path(host_env: &BTreeMap<String, String>) -> String {
    let home = shell_home_dir(host_env);
    let path = std::env::var("PATH").ok();
    shell_exec_path(path.as_deref(), home.as_deref()).unwrap_or_else(default_system_path)
}
