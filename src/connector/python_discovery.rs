fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn python_candidates(runtime_config: &RuntimeConfig) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(value) = runtime_config
        .python_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        push_unique_path_entry(&mut candidates, PathBuf::from(value));
    }
    if let Some(value) = env::var("BRIDGE_AGENT_PYTHON")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        push_unique_path_entry(&mut candidates, PathBuf::from(value));
    }

    #[cfg(windows)]
    for path in windows_python_launcher_candidates() {
        push_unique_path_entry(&mut candidates, path);
    }

    for path in find_python_commands_in_paths() {
        if discovered_python_candidate_is_allowed(&path) {
            push_unique_path_entry(&mut candidates, path);
        }
    }

    for directory in [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/anaconda3/bin"),
    ] {
        append_python_commands_from_dir(&mut candidates, &directory);
    }

    #[cfg(windows)]
    append_windows_python_installations(&mut candidates);

    #[cfg(target_os = "macos")]
    append_macos_python_installations(&mut candidates);

    candidates
}

#[cfg(target_os = "macos")]
fn discovered_python_candidate_is_allowed(path: &Path) -> bool {
    // Apple's /usr/bin/python3 is a developer-tool shim. Executing it on a
    // normal end-user Mac can open the Command Line Tools installer, so it
    // is never an automatically discovered Connector runtime. An explicit
    // runtime.python_path or BRIDGE_AGENT_PYTHON override remains authoritative.
    !path.starts_with("/usr/bin")
}

#[cfg(not(target_os = "macos"))]
fn discovered_python_candidate_is_allowed(_path: &Path) -> bool {
    true
}

fn find_python_commands_in_paths() -> Vec<PathBuf> {
    let mut search_paths = Vec::new();
    if let Ok(path) = env::var("PATH") {
        append_split_path(&mut search_paths, Some(&path));
    }
    if let Some(path) = current_user_command_path() {
        append_split_path(&mut search_paths, Some(&path));
    }
    let mut candidates = Vec::new();
    for path in search_paths {
        append_python_commands_from_dir(&mut candidates, &path);
    }
    candidates
}

fn append_python_commands_from_dir(candidates: &mut Vec<PathBuf>, directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_python_executable_name)
                && path.is_file()
        })
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        push_unique_path_entry(candidates, path);
    }
}

fn is_python_executable_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let normalized = normalized.strip_suffix(".exe").unwrap_or(&normalized);
    if matches!(normalized, "python" | "python3") {
        return true;
    }
    normalized.strip_prefix("python").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.chars().any(|ch| ch.is_ascii_digit())
            && suffix.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
    })
}

#[cfg(windows)]
fn windows_python_launcher_candidates() -> Vec<PathBuf> {
    let mut launcher = Command::new("py");
    configure_connector_command(&mut launcher);
    let Ok(output) = launcher.arg("-0p").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_windows_python_launcher_paths(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(windows, test))]
fn parse_windows_python_launcher_paths(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let remainder = line.split_once(char::is_whitespace)?.1.trim_start();
            let remainder = remainder
                .strip_prefix('*')
                .unwrap_or(remainder)
                .trim_start();
            (!remainder.is_empty()).then(|| PathBuf::from(remainder))
        })
        .collect()
}

#[cfg(windows)]
fn append_windows_python_installations(candidates: &mut Vec<PathBuf>) {
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        append_python_installation_subdirs(
            candidates,
            &PathBuf::from(local_app_data)
                .join("Programs")
                .join("Python"),
        );
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(program_files) = env::var_os(variable) {
            append_python_installation_subdirs(candidates, &PathBuf::from(program_files));
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn append_python_installation_subdirs(candidates: &mut Vec<PathBuf>, root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut directories = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().starts_with("python"))
        })
        .collect::<Vec<_>>();
    directories.sort();
    for directory in directories {
        append_python_commands_from_dir(candidates, &directory);
        append_python_commands_from_dir(candidates, &directory.join("bin"));
    }
}

#[cfg(target_os = "macos")]
fn append_macos_python_installations(candidates: &mut Vec<PathBuf>) {
    let framework_versions = Path::new("/Library/Frameworks/Python.framework/Versions");
    if let Ok(entries) = fs::read_dir(framework_versions) {
        let mut directories = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        directories.sort();
        for directory in directories {
            append_python_commands_from_dir(candidates, &directory.join("bin"));
        }
    }
    for root in ["/opt/homebrew/opt", "/usr/local/opt"] {
        append_python_installation_subdirs(candidates, Path::new(root));
    }
}

fn python_matches_requirement(
    candidate: &Path,
    requires_python: Option<&VersionSpecifiers>,
) -> bool {
    let Some(version) = python_version(candidate) else {
        return false;
    };
    python_version_matches_requirement(&version, requires_python)
}

fn python_version_matches_requirement(
    version: &Version,
    requires_python: Option<&VersionSpecifiers>,
) -> bool {
    requires_python
        .map(|requirement| requirement.contains(version))
        .unwrap_or(true)
}

fn python_version(candidate: &Path) -> Option<Version> {
    let mut command = Command::new(candidate);
    configure_connector_command(&mut command);
    let output = command.arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr)
    } else {
        String::from_utf8_lossy(&output.stdout)
    };
    parse_python_version(&text)
}

fn parse_python_version(value: &str) -> Option<Version> {
    let version = value
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))?;
    Version::from_str(version).ok()
}
