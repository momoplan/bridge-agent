// Reconcile the installed version on every startup, including after app-ID migration.
// A directory's existence says nothing about its executable or update provenance.
fn reconcile_bundled_release(state: &ManagedToolState, bundled: &Path) -> Result<bool> {
    let Ok(version) = validate_cli(bundled, None) else {
        return Ok(false);
    };
    let newer = version_is_newer(&version, &state.active_version)?;
    if !newer && (version != state.active_version || state.install_source.is_some()) {
        return Ok(false);
    }
    let selection = bundled_market_source(bundled, &version)?;
    if !newer && selection.is_none() {
        return Ok(false);
    }
    // For an orphaned installation at the same version, adopt the bundled bytes
    // as well as their source. A version string alone cannot prove provenance.
    import_binary(
        bundled,
        &version,
        if newer {
            "bundled-upgrade"
        } else {
            "bundled-source-repair"
        },
        None,
        selection,
    )?;
    Ok(true)
}

// An update subscription is not proof of where an existing binary came from.
// Orphaned legacy versions use the distribution pinned by this client; existing
// explicit sources keep ownership of their update stream.
fn managed_update_source(
    state: &ManagedToolState,
    bundled: Option<&Path>,
) -> Result<Option<local_app_contract::InstallSource>> {
    if let Some(selection) = &state.install_source {
        return Ok(Some(selection.clone()));
    }
    let Some(bundled) = bundled else {
        return Ok(None);
    };
    let Ok(version) = validate_cli(bundled, None) else {
        return Ok(None);
    };
    bundled_market_source(bundled, &version)
}
