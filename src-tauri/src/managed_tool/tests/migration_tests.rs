use super::*;

struct TestInstallation {
    _guard: std::sync::MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
    old_root: Option<std::ffi::OsString>,
    old_bin: Option<std::ffi::OsString>,
}

impl TestInstallation {
    fn new() -> Self {
        let guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let old_root = std::env::var_os("BAIJIMU_MANAGED_TOOL_ROOT");
        let old_bin = std::env::var_os("BAIJIMU_MANAGED_BIN_DIR");
        std::env::set_var(
            "BAIJIMU_MANAGED_TOOL_ROOT",
            temp.path().join("apps").join(TOOL_ID),
        );
        std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", temp.path().join("bin"));
        Self {
            _guard: guard,
            temp,
            old_root,
            old_bin,
        }
    }

    fn bundled(&self, version: &str) -> PathBuf {
        let binary = self.temp.path().join(format!("bundled-{version}"));
        write_fake_cli(&binary, version);
        let sidecar = binary.with_file_name(format!("bundled-{version}.market.json"));
        fs::write(
            sidecar,
            serde_json::to_vec(&market_source(version)).unwrap(),
        )
        .unwrap();
        binary
    }
}

impl Drop for TestInstallation {
    fn drop(&mut self) {
        for (name, old) in [
            ("BAIJIMU_MANAGED_TOOL_ROOT", &self.old_root),
            ("BAIJIMU_MANAGED_BIN_DIR", &self.old_bin),
        ] {
            if let Some(value) = old {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}

fn market_source(version: &str) -> local_app_contract::InstallSource {
    serde_json::from_value(serde_json::json!({
        "kind":"market", "marketKey":"test-market",
        "listingId":"00000000-0000-0000-0000-000000000001",
        "version":version,
        "source":{"application":{"environmentKey":"test-source","appId":TOOL_ID},"version":version}
    }))
    .unwrap()
}

#[test]
fn migrated_directory_does_not_prevent_upgrade_or_market_source_installation() {
    let fixture = TestInstallation::new();
    let old = fixture.bundled("0.1.0");
    import_binary(&old, "0.1.0", "legacy-download", None, None).unwrap();
    assert!(managed_root().is_dir());
    assert!(!managed_root()
        .parent()
        .unwrap()
        .join("com.baijimu.cli")
        .exists());
    let bundled = fixture.bundled("0.2.0");

    let status = bootstrap_bundled(Some(&bundled)).unwrap();
    assert_eq!(status.installed_version.as_deref(), Some("0.2.0"));
    assert_eq!(status.previous_version.as_deref(), Some("0.1.0"));
    assert_eq!(status.install_source, Some(market_source("0.2.0")));
    assert_eq!(validate_cli(&launcher_path(), None).unwrap(), "0.2.0");
    let persisted = fs::read(state_path()).unwrap();
    bootstrap_bundled(Some(&bundled)).unwrap();
    assert_eq!(fs::read(state_path()).unwrap(), persisted);
    let previous = rollback().unwrap();
    assert_eq!(previous.installed_version.as_deref(), Some("0.1.0"));
    assert!(previous.install_source.is_none());
}

#[test]
fn same_version_repair_adopts_bundled_bytes_and_preserves_rollback_source() {
    let fixture = TestInstallation::new();
    let previous = fixture.bundled("0.1.0");
    bootstrap_bundled(Some(&previous)).unwrap();
    let bundled = fixture.bundled("0.2.0");
    let orphan = fixture.temp.path().join("orphan");
    write_fake_cli(&orphan, "0.2.0");
    let mut bytes = fs::read(&orphan).unwrap();
    bytes.extend_from_slice(b"# same version, different bytes\n");
    fs::write(&orphan, bytes).unwrap();
    import_binary(&orphan, "0.2.0", "legacy-download", None, None).unwrap();
    let installed_at = load_state().unwrap().unwrap().installed_at_epoch_ms;

    let status = bootstrap_bundled(Some(&bundled)).unwrap();
    assert_eq!(status.install_source, Some(market_source("0.2.0")));
    assert_eq!(
        fs::read(launcher_path()).unwrap(),
        fs::read(bundled).unwrap()
    );
    let state = load_state().unwrap().unwrap();
    assert_eq!(state.installed_at_epoch_ms, installed_at);
    assert_eq!(state.previous_version.as_deref(), Some("0.1.0"));
    assert_eq!(state.previous_install_source, Some(market_source("0.1.0")));
    assert_eq!(
        rollback().unwrap().install_source,
        Some(market_source("0.1.0"))
    );
}

#[test]
fn older_bundle_neither_downgrades_nor_claims_a_newer_orphan() {
    let fixture = TestInstallation::new();
    let newer = fixture.bundled("0.3.0");
    import_binary(&newer, "0.3.0", "external-launcher", None, None).unwrap();
    let old = fixture.bundled("0.2.0");
    let persisted = fs::read(state_path()).unwrap();
    let status = bootstrap_bundled(Some(&old)).unwrap();
    assert_eq!(status.installed_version.as_deref(), Some("0.3.0"));
    assert!(status.install_source.is_none());
    assert_eq!(status.update_source, Some(market_source("0.2.0")));
    assert_eq!(fs::read(state_path()).unwrap(), persisted);
}

#[test]
fn same_version_with_an_existing_source_is_not_reassigned() {
    let fixture = TestInstallation::new();
    let bundled = fixture.bundled("0.2.0");
    let mut selection = market_source("0.2.0");
    if let local_app_contract::InstallSource::Market { market_key, .. } = &mut selection {
        *market_key = serde_json::from_value(serde_json::json!("another-market")).unwrap();
    }
    import_binary(&bundled, "0.2.0", "market", None, Some(selection.clone())).unwrap();
    let persisted = fs::read(state_path()).unwrap();
    assert_eq!(
        bootstrap_bundled(Some(&bundled)).unwrap().install_source,
        Some(selection)
    );
    assert_eq!(fs::read(state_path()).unwrap(), persisted);
}

#[test]
fn mismatched_bundled_source_fails_without_changing_installed_state() {
    let fixture = TestInstallation::new();
    let bundled = fixture.bundled("0.2.0");
    import_binary(&bundled, "0.2.0", "legacy-download", None, None).unwrap();
    fs::write(
        fixture.temp.path().join("bundled-0.2.0.market.json"),
        serde_json::to_vec(&market_source("0.3.0")).unwrap(),
    )
    .unwrap();
    let persisted = fs::read(state_path()).unwrap();
    assert!(bootstrap_bundled(Some(&bundled))
        .unwrap_err()
        .to_string()
        .contains("does not match its immutable version"));
    assert_eq!(fs::read(state_path()).unwrap(), persisted);
    assert_eq!(validate_cli(&launcher_path(), None).unwrap(), "0.2.0");
}
