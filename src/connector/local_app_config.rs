fn local_app_from_services(
    manifest: &ConnectorManifest,
    services: &[ServiceConfig],
    registrations: &[ServiceRegistration],
) -> Result<LocalAppConfig> {
    let mut method_names = BTreeSet::new();
    let mut event_names = BTreeSet::new();
    let mut methods = Vec::new();
    let mut events = Vec::new();

    for service in services {
        for method in &service.methods {
            if !method_names.insert(method.name.clone()) {
                bail!(
                    "connector `{}` declares duplicate local app method `{}` across service registrations",
                    manifest.app_id,
                    method.name
                );
            }
            methods.push(method.clone());
        }
    }
    for registration in registrations {
        for event in &registration.local_app_events {
            if !event_names.insert(event.name.clone()) {
                bail!(
                    "connector `{}` declares duplicate local app event `{}` across service registrations",
                    manifest.app_id,
                    event.name
                );
            }
            events.push(event.clone());
        }
    }

    if methods.is_empty() && events.is_empty() {
        bail!(
            "connector `{}` must declare at least one local app method or event",
            manifest.app_id
        );
    }

    Ok(LocalAppConfig {
        app_id: manifest.app_id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        enabled: true,
        health_check: common_lifecycle_value(
            services
                .iter()
                .filter_map(|service| service.health_check.as_ref()),
            &manifest.app_id,
            "healthCheck",
        )?,
        start_command: common_lifecycle_value(
            services
                .iter()
                .filter_map(|service| service.start_command.as_ref()),
            &manifest.app_id,
            "startCommand",
        )?,
        stop_command: common_lifecycle_value(
            services
                .iter()
                .filter_map(|service| service.stop_command.as_ref()),
            &manifest.app_id,
            "stopCommand",
        )?,
        methods,
        events,
    })
}

fn common_lifecycle_value<'a, T>(
    values: impl Iterator<Item = &'a T>,
    app_id: &str,
    field: &str,
) -> Result<Option<T>>
where
    T: Clone + Serialize + 'a,
{
    let values = values.collect::<Vec<_>>();
    let Some(first) = values.first() else {
        return Ok(None);
    };
    let expected = serde_json::to_value(first)?;
    if values
        .iter()
        .skip(1)
        .any(|candidate| serde_json::to_value(candidate).ok().as_ref() != Some(&expected))
    {
        bail!(
            "connector `{app_id}` declares conflicting `{field}` values; lifecycle belongs to the local app and must be identical across registrations"
        );
    }
    Ok(Some((*first).clone()))
}

fn upsert_local_app(
    local_apps: &mut Vec<LocalAppConfig>,
    local_app: LocalAppConfig,
    replace: bool,
) -> Result<()> {
    match local_apps
        .iter()
        .position(|candidate| candidate.app_id == local_app.app_id)
    {
        Some(index) if replace => {
            local_apps[index] = local_app;
            Ok(())
        }
        Some(_) => bail!(
            "local app `{}` already exists; pass --replace to overwrite",
            local_app.app_id
        ),
        None => {
            local_apps.push(local_app);
            Ok(())
        }
    }
}

fn upsert_synced_local_app(local_apps: &mut Vec<LocalAppConfig>, mut local_app: LocalAppConfig) {
    match local_apps
        .iter()
        .position(|candidate| candidate.app_id == local_app.app_id)
    {
        Some(index) => {
            local_app.enabled = local_apps[index].enabled;
            local_apps[index] = local_app;
        }
        None => local_apps.push(local_app),
    }
}

fn cleanup_legacy_connector_autostarts_for_manifest(manifest: &ConnectorManifest) {
    for label in legacy_autostart_labels_for_manifest(manifest) {
        cleanup_legacy_autostart_label(&label);
    }
}

fn legacy_autostart_labels_for_manifest(manifest: &ConnectorManifest) -> Vec<String> {
    let mut labels = manifest.legacy_autostart_labels.clone();
    // WeChat Connector <= 0.3.x installed this fixed LaunchAgent label but did
    // not record it in connector.json. Keep the migration explicit so an
    // upgraded Bridge Agent can remove the old unsigned Python launcher before
    // the Connector package itself is upgraded.
    if manifest.app_id == "com.baijimu.connector.wechat"
        && !labels
            .iter()
            .any(|label| label == "com.baijimu.wechat-bridge-collector")
    {
        labels.push("com.baijimu.wechat-bridge-collector".to_string());
    }
    for value in manifest.hooks.values() {
        if value.contains("install-autostart") {
            if !labels.contains(&manifest.app_id) {
                labels.push(manifest.app_id.clone());
            }
            break;
        }
    }
    labels
}

fn cleanup_legacy_autostart_label(label: &str) {
    #[cfg(target_os = "macos")]
    {
        let label = label.trim();
        if label.is_empty() {
            return;
        }
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(uid) = uid {
            let target = format!("gui/{uid}/{label}");
            let _ = Command::new("launchctl")
                .args(["bootout", &target])
                .output();
        }

        if let Some(home) = env::var_os("HOME") {
            let plist = PathBuf::from(home)
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{label}.plist"));
            match fs::remove_file(&plist) {
                Ok(()) => tracing::info!("removed legacy connector autostart {}", plist.display()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(
                    "failed to remove legacy connector autostart {}: {err:#}",
                    plist.display()
                ),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = label;
    }
}
