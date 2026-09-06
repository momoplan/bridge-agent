fn ensure_connector_event_token(app_id: &str) -> Result<PathBuf> {
    let path = connector_event_token_path(app_id)?;
    if path.exists() {
        return Ok(path);
    }
    let parent = path
        .parent()
        .context("connector event token path has no parent")?;
    fs::create_dir_all(parent)?;
    let token = format!(
        "bjm_evt_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    fs::write(&path, format!("{token}\n"))
        .with_context(|| format!("failed to write connector event token {}", path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(path)
}

fn ensure_connector_management_token(app_id: &str) -> Result<PathBuf> {
    let path = connector_management_token_path(app_id)?;
    if path.exists() {
        let token = fs::read_to_string(&path).with_context(|| {
            format!(
                "failed to read connector management token {}",
                path.display()
            )
        })?;
        if token.trim().len() < 32 {
            bail!("connector management token is invalid: {}", path.display());
        }
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        return Ok(path);
    }
    let parent = path
        .parent()
        .context("connector management token path has no parent")?;
    fs::create_dir_all(parent)?;
    let token = format!(
        "bjm_app_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    fs::write(&path, format!("{token}\n")).with_context(|| {
        format!(
            "failed to write connector management token {}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(path)
}

fn ensure_connector_asset_upload_token(app_id: &str) -> Result<PathBuf> {
    let path = connector_asset_upload_token_path(app_id)?;
    if path.exists() {
        return Ok(path);
    }
    let parent = path
        .parent()
        .context("connector asset upload token path has no parent")?;
    fs::create_dir_all(parent)?;
    let token = format!(
        "bjm_asset_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    fs::write(&path, format!("{token}\n")).with_context(|| {
        format!(
            "failed to write connector asset upload token {}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(path)
}

fn constant_time_text_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn sanitize_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
