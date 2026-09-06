fn collect_headers(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(key, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (key.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn decode_response_body(bytes: &[u8], content_type: &str) -> Value {
    if content_type.contains("application/json") {
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).trim().to_string()))
    } else {
        Value::String(String::from_utf8_lossy(bytes).trim().to_string())
    }
}

fn query_pairs_from_json(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), scalar_to_query_string(value)))
            .collect(),
        _ => Vec::new(),
    }
}

fn scalar_to_query_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

pub fn sanitize_env(env: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut base = BTreeMap::new();
    for key in shell_env_passthrough_keys() {
        if let Ok(value) = std::env::var(key) {
            base.insert(key.to_string(), value);
        }
    }

    for (key, value) in env.into_iter().filter(|(key, _)| {
        key.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }) {
        base.insert(key, value);
    }

    let home = shell_home_dir(&base);
    let path = base
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok());
    base.insert(
        "PATH".to_string(),
        shell_exec_path(path.as_deref(), home.as_deref()).unwrap_or_else(default_system_path),
    );

    base
}

fn shell_env_passthrough_keys() -> &'static [&'static str] {
    if cfg!(windows) {
        &[
            "APPDATA",
            "COMSPEC",
            "HOME",
            "HOMEDRIVE",
            "HOMEPATH",
            "LANG",
            "LOCALAPPDATA",
            "NUMBER_OF_PROCESSORS",
            "OS",
            "PATHEXT",
            "PROCESSOR_ARCHITECTURE",
            "PROCESSOR_IDENTIFIER",
            "PROCESSOR_LEVEL",
            "PROCESSOR_REVISION",
            "PROGRAMDATA",
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "PROGRAMW6432",
            "SYSTEMDRIVE",
            "SYSTEMROOT",
            "TEMP",
            "TMP",
            "USERDOMAIN",
            "USERNAME",
            "USERPROFILE",
            "WINDIR",
        ]
    } else {
        &[
            "HOME", "LANG", "LC_ALL", "LOGNAME", "SHELL", "TMPDIR", "USER",
        ]
    }
}

fn shell_home_dir(base: &BTreeMap<String, String>) -> Option<PathBuf> {
    base.get("HOME")
        .or_else(|| base.get("USERPROFILE"))
        .map(PathBuf::from)
}

fn shell_exec_path(current_path: Option<&str>, home: Option<&Path>) -> Option<String> {
    let mut paths = Vec::new();
    for candidate in shell_toolchain_path_candidates(home) {
        push_path_if_present(&mut paths, candidate);
    }

    #[cfg(windows)]
    {
        for registry_path in windows_registry_path_values() {
            for path in std::env::split_paths(&registry_path) {
                push_path_if_present(&mut paths, path);
            }
        }
    }

    if let Some(current_path) = current_path {
        for path in std::env::split_paths(current_path) {
            push_path_if_present(&mut paths, path);
        }
    }

    if paths.is_empty() {
        return None;
    }

    std::env::join_paths(paths)
        .ok()
        .map(|value| value.to_string_lossy().into_owned())
}

fn shell_toolchain_path_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(home) = home {
        for relative in [
            ".volta/bin",
            ".pyenv/shims",
            ".asdf/shims",
            ".local/share/mise/shims",
            ".mise/shims",
            ".cargo/bin",
            ".bun/bin",
            ".deno/bin",
            ".local/bin",
            "bin",
            "miniconda3/bin",
            "anaconda3/bin",
        ] {
            candidates.push(home.join(relative));
        }

        #[cfg(windows)]
        {
            for relative in [
                "scoop/shims",
                "scoop/apps/git/current/cmd",
                "scoop/apps/git/current/bin",
                "AppData/Local/Programs/Git/cmd",
                "AppData/Local/Programs/Git/bin",
                "AppData/Local/Programs/Git/usr/bin",
            ] {
                candidates.push(home.join(relative));
            }
        }

        candidates.extend(versioned_node_bin_dirs(&home.join(".nvm/versions/node")));
        candidates.extend(versioned_node_bin_dirs(&home.join(".fnm/node-versions")));
        candidates.extend(versioned_node_bin_dirs(
            &home.join(".local/share/fnm/node-versions"),
        ));
    }

    #[cfg(windows)]
    {
        for env_key in ["PROGRAMFILES", "PROGRAMW6432", "PROGRAMFILES(X86)"] {
            if let Ok(program_files) = std::env::var(env_key) {
                let root = PathBuf::from(program_files).join("Git");
                candidates.push(root.join("cmd"));
                candidates.push(root.join("bin"));
                candidates.push(root.join("usr/bin"));
                candidates.push(root.join("mingw64/bin"));
            }
        }
        if let Ok(program_data) = std::env::var("PROGRAMDATA") {
            candidates.push(PathBuf::from(program_data).join("chocolatey/bin"));
        }
    }

    for absolute in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/opt/local/bin",
        "/opt/local/sbin",
        "/opt/anaconda3/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        candidates.push(PathBuf::from(absolute));
    }

    candidates
}

fn versioned_node_bin_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut dirs = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| {
            let version = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(node_version_key)
                .unwrap_or_default();
            let bin = path.join("bin");
            let fnm_bin = path.join("installation/bin");
            (version, bin, fnm_bin)
        })
        .collect::<Vec<_>>();

    dirs.sort_by(|left, right| right.0.cmp(&left.0));

    dirs.into_iter()
        .flat_map(|(_, bin, fnm_bin)| [bin, fnm_bin])
        .collect()
}

fn node_version_key(raw: &str) -> (u64, u64, u64, String) {
    let trimmed = raw.trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|value| {
            value
                .chars()
                .take_while(|char| char.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0);
    (major, minor, patch, raw.to_string())
}

fn push_path_if_present(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidate.is_dir() {
        return;
    }

    if !paths.iter().any(|path| path == &candidate) {
        paths.push(candidate);
    }
}

#[cfg(windows)]
fn windows_registry_path_values() -> Vec<String> {
    [
        r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        r"HKCU\Environment",
    ]
    .into_iter()
    .filter_map(windows_registry_path_value)
    .collect()
}

#[cfg(windows)]
fn windows_registry_path_value(key: &str) -> Option<String> {
    let mut command = StdCommand::new(windows_system32_exe("reg.exe"));
    command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    let output = command.args(["query", key, "/v", "Path"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(parse_reg_path_line)
}

#[cfg(windows)]
fn parse_reg_path_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with("Path") {
        return None;
    }

    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    let value_index = parts
        .iter()
        .position(|part| *part == "REG_SZ" || *part == "REG_EXPAND_SZ")?
        + 1;
    if value_index >= parts.len() {
        return None;
    }
    let raw = parts[value_index..].join(" ");
    Some(expand_windows_env_vars(&raw))
}

#[cfg(windows)]
fn expand_windows_env_vars(value: &str) -> String {
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
        match std::env::var(key) {
            Ok(env_value) => expanded.push_str(&env_value),
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

#[cfg(windows)]
fn windows_system32_exe(file_name: &str) -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join(file_name)
}

fn default_system_path() -> String {
    if cfg!(windows) {
        let root = std::env::var("SystemRoot")
            .or_else(|_| std::env::var("WINDIR"))
            .unwrap_or_else(|_| r"C:\Windows".to_string());
        std::env::join_paths([
            PathBuf::from(&root).join("System32"),
            PathBuf::from(&root),
            PathBuf::from(&root).join("System32/WindowsPowerShell/v1.0"),
        ])
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
    } else {
        "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    }
}

pub fn is_command_allowed(command: &str, allowlist: &[String]) -> bool {
    if allowlist.iter().any(|allowed| allowed.trim() == "*") {
        return true;
    }
    let name = Path::new(command)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(command);
    allowlist
        .iter()
        .any(|allowed| allowed == command || allowed == name)
}

pub fn resolve_cwd(root_dir: &Path, requested: Option<&str>) -> Result<PathBuf> {
    let candidate = match requested {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                root_dir.join(path)
            }
        }
        None => root_dir.to_path_buf(),
    };

    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("failed to resolve cwd {}", candidate.display()))?;
    if !canonical.starts_with(root_dir) {
        bail!(
            "cwd {} escapes root dir {}",
            canonical.display(),
            root_dir.display()
        );
    }
    Ok(canonical)
}
