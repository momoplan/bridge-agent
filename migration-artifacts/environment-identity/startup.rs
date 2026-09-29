use anyhow::{ensure, Context, Result};
use bridge_agent::ServiceStartCommand;
use serde::Deserialize;
use std::fs;
use std::path::Path;

// Only lifecycle commands are read before conversion. Provenance stays owned by source.rs.
#[derive(Deserialize)]
struct StartupConfig {
    #[serde(default)]
    local_apps: Vec<StartupApplication>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartupApplication {
    app_id: String,
    stop_command: Option<ServiceStartCommand>,
}

pub(super) fn stop_applications(config: &Path) -> Result<()> {
    ensure!(config.is_absolute(), "startup config must be absolute");
    if !config.try_exists()? {
        return Ok(());
    }
    let config: StartupConfig = serde_json::from_slice(&fs::read(config)?)
        .context("cannot read application shutdown commands before migration")?;
    for app in config.local_apps {
        let Some(command) = app.stop_command else {
            continue;
        };
        let result = bridge_agent::connector::run_lifecycle_command(&app.app_id, &command)?;
        // Do not echo command output, which may contain application credentials.
        ensure!(
            result.exit_code == Some(0),
            "application {} shutdown failed (exit {:?}); installation records were not changed",
            app.app_id,
            result.exit_code
        );
    }
    Ok(())
}
