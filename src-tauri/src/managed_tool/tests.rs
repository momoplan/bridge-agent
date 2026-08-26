use super::*;

#[cfg(unix)]
use std::sync::Mutex;

#[cfg(unix)]
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn checksum_accepts_prefixed_sha256() {
    let value = "a".repeat(64);
    assert_eq!(normalize_sha256(&format!("sha256:{value}")).unwrap(), value);
}

#[test]
fn windows_user_path_is_prepended_without_losing_existing_entries() {
    let merged = merge_windows_user_path_with(
        r"C:\Windows\System32;C:\Tools;",
        Path::new(r"C:\Users\Ada\AppData\Local\Baijimu\bin"),
        |_| None,
    );
    assert_eq!(
        merged,
        r"C:\Users\Ada\AppData\Local\Baijimu\bin;C:\Windows\System32;C:\Tools;"
    );
}

#[test]
fn windows_user_path_is_idempotent_and_deduplicated_case_insensitively() {
    let launcher = Path::new(r"C:\Users\Ada\AppData\Local\Baijimu\bin");
    let merged = merge_windows_user_path_with(
        r#"c:/users/ada/appdata/local/baijimu/bin/;C:\Tools;"C:\USERS\ADA\APPDATA\LOCAL\BAIJIMU\BIN""#,
        launcher,
        |_| None,
    );
    assert_eq!(merged, r"C:\Users\Ada\AppData\Local\Baijimu\bin;C:\Tools");
    assert_eq!(
        merge_windows_user_path_with(&merged, launcher, |_| None),
        merged
    );
}

#[test]
fn windows_user_path_recognizes_environment_variable_entries() {
    let launcher = Path::new(r"C:\Users\Ada\AppData\Local\Baijimu\bin");
    let lookup = |name: &str| match name.to_ascii_uppercase().as_str() {
        "LOCALAPPDATA" => Some(r"C:\Users\Ada\AppData\Local".to_string()),
        _ => None,
    };
    let merged = merge_windows_user_path_with(
        r"%LOCALAPPDATA%\Baijimu\bin;C:\Windows\System32",
        launcher,
        lookup,
    );
    assert_eq!(
        merged,
        r"C:\Users\Ada\AppData\Local\Baijimu\bin;C:\Windows\System32"
    );
    assert!(windows_path_contains_with(
        r"C:\Windows;%LOCALAPPDATA%\Baijimu\bin",
        launcher,
        lookup
    ));
}

#[test]
fn empty_windows_user_path_becomes_only_the_launcher_directory() {
    assert_eq!(
        merge_windows_user_path_with("", Path::new(r"C:\Baijimu\bin"), |_| None),
        r"C:\Baijimu\bin"
    );
}

#[cfg(windows)]
#[test]
fn windows_registry_path_registration_round_trips() {
    struct TestRegistryKey {
        root: RegKey,
        path: String,
    }

    impl Drop for TestRegistryKey {
        fn drop(&mut self) {
            let _ = self.root.delete_subkey_all(&self.path);
        }
    }

    let root = RegKey::predef(HKEY_CURRENT_USER);
    let test_key = TestRegistryKey {
        root,
        path: format!(
            r"Software\Baijimu\BridgeAgentTests\{}",
            uuid::Uuid::new_v4()
        ),
    };
    let (environment, _) = test_key.root.create_subkey(&test_key.path).unwrap();
    environment
        .set_raw_value(
            "Path",
            &RegValue {
                bytes: r"C:\Windows\System32"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                vtype: REG_EXPAND_SZ,
            },
        )
        .unwrap();

    let launcher = Path::new(r"C:\Users\Ada\AppData\Local\Baijimu\bin");
    assert!(ensure_windows_user_path_in_registry(&environment, launcher).unwrap());
    assert!(!ensure_windows_user_path_in_registry(&environment, launcher).unwrap());
    let registered = String::from_reg_value(&environment.get_raw_value("Path").unwrap()).unwrap();
    assert_eq!(
        registered,
        r"C:\Users\Ada\AppData\Local\Baijimu\bin;C:\Windows\System32"
    );
}

#[test]
fn zip_extracts_explicit_cli_path() {
    let bytes = zip_with_entry(&format!("bin/{}", binary_name()), b"test-cli");
    assert_eq!(
        extract_binary(
            &bytes,
            "https://example.test/baijimu.zip",
            Some(&format!("bin/{}", binary_name()))
        )
        .unwrap(),
        b"test-cli"
    );
}

#[test]
fn zip_extracts_windows_separator_entry_with_portable_explicit_path() {
    let bytes = zip_with_entry(r"bin\baijimu.exe", b"windows-cli");
    assert_eq!(
        extract_binary(
            &bytes,
            "https://example.test/baijimu-windows.zip",
            Some("bin/baijimu.exe")
        )
        .unwrap(),
        b"windows-cli"
    );
}

#[test]
fn zip_rejects_ambiguous_normalized_explicit_path() {
    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file::<_, ()>("bin/baijimu.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"portable-cli").unwrap();
    writer
        .start_file::<_, ()>(r"bin\baijimu.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"windows-cli").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let error = extract_binary(
        &bytes,
        "https://example.test/baijimu-windows.zip",
        Some("bin/baijimu.exe"),
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("multiple entries matching bin/baijimu.exe"));
}

fn zip_with_entry(path: &str, contents: &[u8]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file::<_, ()>(path, zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(contents).unwrap();
    writer.finish().unwrap().into_inner()
}

#[cfg(unix)]
#[test]
fn managed_bootstrap_never_downgrades_and_supports_rollback() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("managed");
    let bin = temp.path().join("bin");
    std::env::set_var("BAIJIMU_MANAGED_TOOL_ROOT", &root);
    std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", &bin);

    let bundled = temp.path().join("baijimu-bundled");
    let newer = temp.path().join("baijimu-newer");
    write_fake_cli(&bundled, "0.1.0");
    write_fake_cli(&newer, "0.2.0");

    let initial = bootstrap_bundled(Some(&bundled)).unwrap();
    assert_eq!(initial.installed_version.as_deref(), Some("0.1.0"));

    import_binary(&newer, "0.2.0", "test-update", None).unwrap();
    let after_restart = bootstrap_bundled(Some(&bundled)).unwrap();
    assert_eq!(after_restart.installed_version.as_deref(), Some("0.2.0"));
    assert_eq!(after_restart.previous_version.as_deref(), Some("0.1.0"));

    let rolled_back = rollback().unwrap();
    assert_eq!(rolled_back.installed_version.as_deref(), Some("0.1.0"));
    assert_eq!(rolled_back.previous_version.as_deref(), Some("0.2.0"));

    std::env::remove_var("BAIJIMU_MANAGED_TOOL_ROOT");
    std::env::remove_var("BAIJIMU_MANAGED_BIN_DIR");
}

#[cfg(unix)]
#[test]
fn managed_bootstrap_adopts_newer_launcher_and_bundled_cli() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("managed");
    let bin = temp.path().join("bin");
    std::env::set_var("BAIJIMU_MANAGED_TOOL_ROOT", &root);
    std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", &bin);

    let initial = temp.path().join("baijimu-initial");
    let launcher = bin.join(binary_name());
    let bundled = temp.path().join("baijimu-bundled");
    write_fake_cli(&initial, "0.1.17");
    write_fake_cli(&bundled, "0.1.23");

    let first = bootstrap_bundled(Some(&initial)).unwrap();
    assert_eq!(first.installed_version.as_deref(), Some("0.1.17"));
    write_fake_cli(&launcher, "0.1.22");

    let upgraded = bootstrap_bundled(Some(&bundled)).unwrap();
    assert_eq!(upgraded.installed_version.as_deref(), Some("0.1.23"));
    assert_eq!(
        validate_cli(&launcher, None).unwrap(),
        upgraded.installed_version.unwrap()
    );
    assert_eq!(upgraded.previous_version.as_deref(), Some("0.1.22"));

    std::env::remove_var("BAIJIMU_MANAGED_TOOL_ROOT");
    std::env::remove_var("BAIJIMU_MANAGED_BIN_DIR");
}

#[cfg(unix)]
#[test]
fn managed_inspect_adopts_externally_updated_launcher() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("managed");
    let bin = temp.path().join("bin");
    std::env::set_var("BAIJIMU_MANAGED_TOOL_ROOT", &root);
    std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", &bin);

    let initial = temp.path().join("baijimu-initial");
    let launcher = bin.join(binary_name());
    write_fake_cli(&initial, "0.1.26");

    let first = bootstrap_bundled(Some(&initial)).unwrap();
    assert_eq!(first.installed_version.as_deref(), Some("0.1.26"));

    write_fake_cli(&launcher, "0.1.31");
    let inspected = inspect(Some(&initial)).unwrap();
    assert_eq!(inspected.state, "ready");
    assert_eq!(inspected.installed_version.as_deref(), Some("0.1.31"));
    assert_eq!(inspected.previous_version.as_deref(), Some("0.1.26"));
    assert_eq!(
        validate_cli(&version_binary_path("0.1.31"), None).unwrap(),
        "0.1.31"
    );

    std::env::remove_var("BAIJIMU_MANAGED_TOOL_ROOT");
    std::env::remove_var("BAIJIMU_MANAGED_BIN_DIR");
}

#[cfg(unix)]
#[test]
fn managed_inspect_repairs_stale_launcher_from_active_version() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("managed");
    let bin = temp.path().join("bin");
    std::env::set_var("BAIJIMU_MANAGED_TOOL_ROOT", &root);
    std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", &bin);

    let initial = temp.path().join("baijimu-initial");
    let launcher = bin.join(binary_name());
    write_fake_cli(&initial, "0.1.26");

    let first = bootstrap_bundled(Some(&initial)).unwrap();
    assert_eq!(first.installed_version.as_deref(), Some("0.1.26"));

    write_fake_cli(&launcher, "0.1.20");
    let inspected = inspect(Some(&initial)).unwrap();
    assert_eq!(inspected.state, "ready");
    assert_eq!(inspected.installed_version.as_deref(), Some("0.1.26"));
    assert_eq!(validate_cli(&launcher, None).unwrap(), "0.1.26");

    std::env::remove_var("BAIJIMU_MANAGED_TOOL_ROOT");
    std::env::remove_var("BAIJIMU_MANAGED_BIN_DIR");
}

#[cfg(unix)]
#[test]
fn managed_dependency_bootstraps_validates_version_and_returns_stable_launcher() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("managed");
    let bin = temp.path().join("bin");
    std::env::set_var("BAIJIMU_MANAGED_TOOL_ROOT", &root);
    std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", &bin);

    let bundled = temp.path().join("baijimu-bundled");
    write_fake_cli(&bundled, "0.2.0");

    let ready = ensure_bundled_dependency_ready(TOOL_ID, "0.1.45", Some(&bundled)).unwrap();
    let launcher = PathBuf::from(&ready.launcher_path);
    assert_eq!(ready.state, "ready");
    assert_eq!(ready.installed_version.as_deref(), Some("0.2.0"));
    assert!(launcher.is_absolute());
    assert!(launcher.is_file());

    let version_error =
        ensure_bundled_dependency_ready(TOOL_ID, "0.3.0", Some(&bundled)).unwrap_err();
    assert!(version_error.to_string().contains("requires version 0.3.0"));
    let id_error =
        ensure_bundled_dependency_ready("com.example.unknown-tool", "0.1.0", Some(&bundled))
            .unwrap_err();
    assert!(id_error.to_string().contains("unsupported managed tool"));

    std::env::remove_var("BAIJIMU_MANAGED_TOOL_ROOT");
    std::env::remove_var("BAIJIMU_MANAGED_BIN_DIR");
}

#[cfg(unix)]
fn write_fake_cli(path: &Path, version: &str) {
    fs::write(
        path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' '{{\"name\":\"baijimu\",\"version\":\"{version}\",\"implementation\":\"rust-native\"}}'\n"
        ),
    )
    .unwrap();
    set_executable(path).unwrap();
}
