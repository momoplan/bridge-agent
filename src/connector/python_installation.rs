fn install_python_project_dependencies(package_path: &Path, python: &Path) -> Result<()> {
    let lock_path = package_path.join("requirements.lock");
    if lock_path.is_file() {
        let mut install = Command::new(python);
        configure_connector_command(&mut install);
        let output = install
            .args([
                "-I",
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                "--requirement",
            ])
            .arg(&lock_path)
            .output()
            .with_context(|| {
                format!(
                    "failed to install locked Python connector dependencies with {}",
                    python.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "failed to install locked Python connector dependencies for {}\nstdout:\n{}\nstderr:\n{}",
                package_path.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(());
    }
    let dependencies = read_python_project_dependencies(package_path)?;
    if dependencies.is_empty() {
        return Ok(());
    }
    let mut install = Command::new(python);
    configure_connector_command(&mut install);
    let output = install
        .args(["-I", "-m", "pip", "install", "--disable-pip-version-check"])
        .args(&dependencies)
        .output()
        .with_context(|| {
            format!(
                "failed to install Python connector dependencies with {}",
                python.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "failed to install Python connector dependencies for {}\nstdout:\n{}\nstderr:\n{}",
            package_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn write_python_project_scripts(
    package_path: &Path,
    env_path: &Path,
    scripts: &BTreeMap<String, String>,
) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }
    let bin_dir = python_bin_dir(env_path);
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    for (script, module) in scripts {
        write_python_project_script(package_path, env_path, script, module)?;
    }
    Ok(())
}

fn write_python_project_script(
    package_path: &Path,
    env_path: &Path,
    script: &str,
    entrypoint: &str,
) -> Result<()> {
    let target = python_script_path(env_path, script);
    let (module, function) = python_entrypoint_parts(entrypoint);
    #[cfg(windows)]
    {
        let python = python_env_executable(env_path);
        let runner = format!(
            "@echo off\r\nset PYTHONNOUSERSITE=1\r\nset PYTHONPATH=\r\n\"{}\" -I -c \"import sys; sys.path.insert(0, r'{}'); from {} import {}; raise SystemExit({}())\" %*\r\n",
            python.display(),
            package_path.display(),
            module,
            function,
            function
        );
        fs::write(&target, runner.as_bytes()).with_context(|| {
            format!(
                "failed to write Python connector script {}",
                target.display()
            )
        })?;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let python = python_env_executable(env_path);
        let runner = format!(
            "#!/bin/sh\nunset PYTHONPATH\nexport PYTHONNOUSERSITE=1\nexec {} -I - \"$@\" <<'PY'\nimport sys\nsys.path.insert(0, {:?})\nfrom {} import {}\nif __name__ == '__main__':\n    raise SystemExit({}())\nPY\n",
            shell_single_quote(&python.display().to_string()),
            package_path.display().to_string(),
            module,
            function,
            function
        );
        fs::write(&target, runner.as_bytes()).with_context(|| {
            format!(
                "failed to write Python connector script {}",
                target.display()
            )
        })?;
        let mut permissions = fs::metadata(&target)
            .with_context(|| format!("failed to inspect {}", target.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&target, permissions)
            .with_context(|| format!("failed to make {} executable", target.display()))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn python_entrypoint_parts(entrypoint: &str) -> (&str, &str) {
    let (module, function) = entrypoint.split_once(':').unwrap_or((entrypoint, "main"));
    let module = module.trim();
    let function = function.trim();
    if function.is_empty() {
        (module, "main")
    } else {
        (module, function)
    }
}

fn mark_python_project_installed(package_path: &Path, env_path: &Path) -> Result<()> {
    let marker = env_path.join(CONNECTOR_PYTHON_ENV_MARKER);
    fs::write(
        &marker,
        format!("package={}\n", package_path.display()).as_bytes(),
    )
    .with_context(|| format!("failed to write {}", marker.display()))?;
    Ok(())
}

fn python_env_executable(env_path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        env_path.join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        env_path.join("bin").join("python")
    }
}

fn python_script_path(env_path: &Path, script: &str) -> PathBuf {
    #[cfg(windows)]
    {
        env_path.join("Scripts").join(format!("{script}.cmd"))
    }
    #[cfg(not(windows))]
    {
        env_path.join("bin").join(script)
    }
}

fn python_bin_dir(env_path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        env_path.join("Scripts")
    }
    #[cfg(not(windows))]
    {
        env_path.join("bin")
    }
}
