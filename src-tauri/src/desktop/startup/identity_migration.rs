fn migrate_installations_before_startup(config_path: &Path) -> anyhow::Result<bool> {
    let config_changed = migrate_legacy_config_before_startup(config_path)?;
    let binary_name = if cfg!(windows) {
        "bridge-agent-environment-identity-migration.exe"
    } else {
        "bridge-agent-environment-identity-migration"
    };
    let binary = bundled_resource_binary_path(binary_name).with_context(|| {
        format!("missing environment identity migration artifact {binary_name}")
    })?;
    let managed_root = managed_tool::managed_root();
    let managed_apps = managed_root
        .parent()
        .context("managed tool root has no parent")?;
    run_identity_migration(
        &binary,
        config_path,
        &bridge_agent::connectors_dir()?,
        managed_apps,
    )?;
    Ok(config_changed)
}

fn run_identity_migration(
    binary: &Path,
    config_path: &Path,
    local_apps: &Path,
    managed_apps: &Path,
) -> anyhow::Result<()> {
    let mut command = Command::new(binary);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .arg("--config-dir")
        .arg(resolve_config_base_dir(config_path))
        .arg("--local-apps-dir")
        .arg(local_apps)
        .arg("--managed-apps-dir")
        .arg(managed_apps)
        .arg("--prepare-startup")
        .arg("--config")
        .arg(config_path)
        .output()
        .with_context(|| format!("failed to start migration artifact {}", binary.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "environment identity migration failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

// Recovery and diagnostics remain available even when installation records cannot be read.
pub(super) fn command_allowed_before_migration(command: &str) -> bool {
    matches!(
        command,
        "frontend_heartbeat"
            | "report_frontend_failure"
            | "open_native_recovery"
            | "app_version"
            | "get_startup_health"
            | "mark_frontend_ready"
            | "restart_in_normal_mode"
            | "open_startup_log"
            | "check_app_update"
            | "install_app_update"
            | "open_app_uninstaller"
            | "desktop_permission_status"
            | "request_desktop_permission"
            | "open_desktop_permission_settings"
            | "open_in_browser"
            | "open_in_edge"
    )
}

#[cfg(test)]
mod identity_startup_tests {
    use super::*;

    #[test]
    fn migration_gate_keeps_recovery_available_and_blocks_business_writers() {
        for command in [
            "install_app_update",
            "get_startup_health",
            "open_startup_log",
            "frontend_heartbeat",
        ] {
            assert!(command_allowed_before_migration(command));
        }
        for command in [
            "load_config",
            "save_config",
            "start_agent",
            "start_connector_app",
            "start_connector_app_install",
            "install_baijimu_cli_update",
            "rollback_baijimu_cli",
        ] {
            assert!(!command_allowed_before_migration(command));
        }
    }
}
