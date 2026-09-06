fn read_python_project_scripts(package_path: &Path) -> Result<BTreeMap<String, String>> {
    let path = package_path.join("pyproject.toml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read Python project metadata {}", path.display()))?;
    let project: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse Python project metadata {}", path.display()))?;
    let mut scripts = BTreeMap::new();
    let Some(table) = project
        .get("project")
        .and_then(|value| value.get("scripts"))
        .and_then(toml::Value::as_table)
    else {
        return Ok(scripts);
    };
    for (name, value) in table {
        let Some(entrypoint) = value.as_str() else {
            continue;
        };
        let entrypoint = entrypoint.trim();
        let Some(module) = entrypoint.split(':').next() else {
            continue;
        };
        let module = module.trim();
        if !name.trim().is_empty() && !module.is_empty() && !entrypoint.is_empty() {
            scripts.insert(name.trim().to_string(), entrypoint.to_string());
        }
    }
    Ok(scripts)
}

fn ensure_python_project_environment(
    package_path: &Path,
    scripts: &BTreeMap<String, String>,
    runtime_config: &RuntimeConfig,
) -> Result<Option<PathBuf>> {
    if scripts.is_empty() {
        return Ok(None);
    }
    let env_path = package_path.join(CONNECTOR_PYTHON_ENV_DIR);
    let python = python_env_executable(&env_path);
    let requires_python = read_python_requires_python(package_path)?;
    let base_python = resolve_python_for_project(requires_python.as_deref(), runtime_config)?;
    if python.exists() && !python_env_uses_base_interpreter(&python, Path::new(&base_python)) {
        fs::remove_dir_all(&env_path).with_context(|| {
            format!(
                "failed to recreate Python connector environment {} after interpreter change",
                env_path.display()
            )
        })?;
    }
    if !python.exists() {
        create_python_env(&env_path, &base_python)?;
    }
    if python_project_install_needed(package_path, &env_path)? {
        install_python_project_dependencies(package_path, &python)?;
        write_python_project_scripts(package_path, &env_path, scripts)?;
        mark_python_project_installed(package_path, &env_path)?;
    } else {
        write_python_project_scripts(package_path, &env_path, scripts)?;
    }
    Ok(Some(env_path))
}

fn python_project_install_needed(package_path: &Path, env_path: &Path) -> Result<bool> {
    let marker = env_path.join(CONNECTOR_PYTHON_ENV_MARKER);
    if !marker.exists() {
        return Ok(true);
    }
    let marker_modified = marker
        .metadata()
        .and_then(|metadata| metadata.modified())
        .with_context(|| format!("failed to inspect {}", marker.display()))?;
    for relative in [
        "pyproject.toml",
        "requirements.lock",
        "setup.py",
        "setup.cfg",
    ] {
        let path = package_path.join(relative);
        if !path.exists() {
            continue;
        }
        let modified = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if modified > marker_modified {
            return Ok(true);
        }
    }
    Ok(false)
}

fn create_python_env(env_path: &Path, base_python: &str) -> Result<()> {
    let parent = env_path
        .parent()
        .with_context(|| format!("failed to resolve parent for {}", env_path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let mut create = Command::new(base_python);
    configure_connector_command(&mut create);
    let output = create
        .args(["-m", "venv"])
        .arg(env_path)
        .output()
        .with_context(|| {
            format!("failed to create Python environment with `{base_python} -m venv`")
        })?;
    if !output.status.success() {
        bail!(
            "failed to create Python connector environment {}\nstdout:\n{}\nstderr:\n{}",
            env_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn python_env_uses_base_interpreter(env_python: &Path, base_python: &Path) -> bool {
    let mut inspect = Command::new(env_python);
    configure_connector_command(&mut inspect);
    let output = inspect
        .args([
            "-I",
            "-c",
            "import os,sys; print(os.path.realpath(sys._base_executable))",
        ])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let current = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let expected = base_python
        .canonicalize()
        .unwrap_or_else(|_| base_python.to_path_buf());
    let current = current.canonicalize().unwrap_or(current);
    current == expected
}

fn read_python_requires_python(package_path: &Path) -> Result<Option<String>> {
    let Some(project) = read_python_project_metadata(package_path)? else {
        return Ok(None);
    };
    Ok(project
        .get("project")
        .and_then(|value| value.get("requires-python"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn read_python_project_dependencies(package_path: &Path) -> Result<Vec<String>> {
    let Some(project) = read_python_project_metadata(package_path)? else {
        return Ok(Vec::new());
    };
    let Some(dependencies) = project
        .get("project")
        .and_then(|value| value.get("dependencies"))
        .and_then(toml::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    Ok(dependencies
        .iter()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn read_python_project_metadata(package_path: &Path) -> Result<Option<toml::Value>> {
    let path = package_path.join("pyproject.toml");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read Python project metadata {}", path.display()))?;
    let project = toml::from_str(&content)
        .with_context(|| format!("failed to parse Python project metadata {}", path.display()))?;
    Ok(Some(project))
}

fn resolve_python_for_project(
    requires_python: Option<&str>,
    runtime_config: &RuntimeConfig,
) -> Result<String> {
    let requirement = requires_python
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let specifiers = requirement
        .map(VersionSpecifiers::from_str)
        .transpose()
        .with_context(|| {
            format!(
                "invalid Connector requires-python `{}`",
                requirement.unwrap_or_default()
            )
        })?;
    for candidate in python_candidates(runtime_config) {
        if python_matches_requirement(&candidate, specifiers.as_ref()) {
            return Ok(candidate.display().to_string());
        }
    }
    if let Some(requirement) = requirement {
        bail!(
            "failed to find a Python interpreter matching Connector requires-python `{requirement}`. Install a compatible Python version or set runtime.python_path to its absolute executable path.",
        )
    }
    bail!(
        "failed to find a usable Python interpreter. Install Python or set runtime.python_path to its absolute executable path.",
    )
}

pub fn inspect_python_runtime(runtime_config: &RuntimeConfig) -> PythonRuntimeStatus {
    let configured_path = runtime_config
        .python_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    for candidate in python_candidates(runtime_config) {
        let Some(version) = python_version(&candidate) else {
            continue;
        };
        let version_text = version.to_string();
        let detected_path = candidate
            .canonicalize()
            .unwrap_or(candidate)
            .display()
            .to_string();
        let message = if configured_path.as_deref().is_some_and(|configured| {
            paths_refer_to_same_file(Path::new(configured), Path::new(&detected_path))
        }) {
            format!(
                "已检测到首选 Python {version_text}。启动 Python Connector 时会按其 requires-python 校验版本。"
            )
        } else if configured_path.is_some() {
            format!(
                "配置的 Python 不可用，已自动发现 Python {version_text}。启动 Python Connector 时会按其 requires-python 选择兼容版本。"
            )
        } else {
            format!(
                "已检测到 Python {version_text}。启动 Python Connector 时会按其 requires-python 选择兼容版本。"
            )
        };
        return PythonRuntimeStatus {
            configured_path,
            detected_path: Some(detected_path),
            version: Some(version_text),
            available: true,
            message,
        };
    }

    PythonRuntimeStatus {
        configured_path,
        detected_path: None,
        version: None,
        available: false,
        message: "未检测到可执行的 Python。需要 Python 的 Connector 会通过 requires-python 声明所需版本。"
            .to_string(),
    }
}
