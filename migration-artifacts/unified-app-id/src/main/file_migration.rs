fn move_app_data(config_dir: &Path, entry: &MigrationEntry) -> Result<()> {
    let source = config_dir
        .join("connector-data")
        .join(&entry.legacy_identity);
    let target = config_dir.join("app-data").join(&entry.app_id);
    if source.exists() && target.exists() {
        return move_once(
            &source,
            &config_dir
                .join("migration-backups")
                .join("unified-app-id")
                .join(ARTIFACT_VERSION)
                .join("superseded")
                .join("connector-data")
                .join(&entry.legacy_identity),
            "superseded legacy application data",
        );
    }
    move_once(&source, &target, "application data")
}

fn archive_package(config_dir: &Path, entry: &MigrationEntry) -> Result<()> {
    let source = config_dir.join("connectors").join(&entry.legacy_identity);
    let target = config_dir
        .join("migration-backups")
        .join("unified-app-id")
        .join(ARTIFACT_VERSION)
        .join("connectors")
        .join(&entry.legacy_identity);
    move_once(&source, &target, "legacy package")
}

fn legacy_config_has_local_apps(config_path: &Path) -> Result<bool> {
    Ok(!read_json::<LegacyAgentConfig>(config_path)?
        .local_apps
        .is_empty())
}

fn directory_has_entries(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    Ok(fs::read_dir(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .next()
        .transpose()?
        .is_some())
}

fn legacy_state_exists(config_dir: &Path, config_path: &Path) -> Result<bool> {
    Ok(legacy_config_has_local_apps(config_path)?
        || directory_has_entries(&config_dir.join("connectors"))?
        || directory_has_entries(&config_dir.join("connector-data"))?)
}

fn legacy_identity_set(config_dir: &Path, config_path: &Path) -> Result<BTreeSet<String>> {
    let mut identities = read_json::<LegacyAgentConfig>(config_path)?
        .local_apps
        .into_iter()
        .map(|app| app.connector_id)
        .collect::<BTreeSet<_>>();
    for root in [
        config_dir.join("connectors"),
        config_dir.join("connector-data"),
    ] {
        if !root.is_dir() {
            continue;
        }
        for item in
            fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
        {
            let item = item?;
            if item.file_type()?.is_dir() {
                identities.insert(item.file_name().to_string_lossy().into_owned());
            }
        }
    }
    for identity in &identities {
        validate_path_segment(identity, "legacy application identity")?;
    }
    Ok(identities)
}
