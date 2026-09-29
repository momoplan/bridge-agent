use std::fs;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};

fn migrate(root: &Path) -> Output {
    Command::new(env!(
        "CARGO_BIN_EXE_bridge-agent-environment-identity-migration"
    ))
    .arg("--config-dir")
    .arg(root.join("config"))
    .arg("--local-apps-dir")
    .arg(root.join("local-apps"))
    .arg("--managed-apps-dir")
    .arg(root.join("managed"))
    .arg("--host-already-stopped")
    .output()
    .unwrap()
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for directory in ["config", "local-apps/example", "managed"] {
        fs::create_dir_all(root.path().join(directory)).unwrap();
    }
    fs::write(
        root.path().join("local-apps/example/install.json"),
        r#"{"installSource":{"kind":"market","marketKey":"legacy","listingId":"00000000-0000-0000-0000-000000000001","version":"1.0.0","source":{"application":{"environmentKey":"author","appId":"example"},"version":"1.0.0"}},"manifest":{}}"#,
    )
    .unwrap();
    root
}

#[test]
fn executable_migrates_with_its_required_directory_arguments() {
    let root = fixture();
    let path = root.path().join("local-apps/example/install.json");
    let original = fs::read(&path).unwrap();
    let output = migrate(root.path());
    assert!(output.status.success(), "{:?}", output);
    let updated = fs::read_to_string(&path).unwrap();
    assert!(!updated.contains("marketKey"));
    assert!(updated.contains("author"));
    assert_eq!(
        fs::read(path.with_extension("json.before-environment-identity-3")).unwrap(),
        original
    );
    assert!(migrate(root.path()).status.success());
}

struct Writer(Child);

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn another_process_in_the_installation_still_blocks_migration() {
    let root = fixture();
    let installation = root.path().join("local-apps/example");
    let path = installation.join("install.json");
    let original = fs::read(&path).unwrap();
    let ready = installation.join("writer-ready");
    let executable = installation.join(if cfg!(windows) {
        "writer.exe"
    } else {
        "writer"
    });
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let mut writer = Writer(
        Command::new(executable)
            .args(["--exact", "installation_writer_process", "--ignored"])
            .current_dir(&installation)
            .env("IDENTITY_MIGRATION_WRITER_READY", &ready)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(writer.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline, "writer did not become ready");
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = migrate(root.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("an application is still using its installation"));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!path
        .with_extension("json.before-environment-identity-3")
        .exists());
}

#[test]
#[ignore = "subprocess fixture started by the process safety test"]
fn installation_writer_process() {
    let ready = std::env::var_os("IDENTITY_MIGRATION_WRITER_READY").unwrap();
    fs::write(ready, b"ready").unwrap();
    let stop = std::env::var_os("IDENTITY_MIGRATION_WRITER_STOP");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if stop.as_ref().is_some_and(|path| Path::new(path).exists()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn startup_migrate(root: &Path) -> Output {
    Command::new(env!(
        "CARGO_BIN_EXE_bridge-agent-environment-identity-migration"
    ))
    .arg("--config-dir")
    .arg(root.join("config"))
    .arg("--local-apps-dir")
    .arg(root.join("local-apps"))
    .arg("--managed-apps-dir")
    .arg(root.join("managed"))
    .arg("--prepare-startup")
    .arg("--config")
    .arg(root.join("config/agent-config.json"))
    .output()
    .unwrap()
}

#[test]
fn first_startup_converts_089_records_before_current_protocol_reads_them() {
    let root = fixture();
    let install = root.path().join("local-apps/example/install.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&install).unwrap()).unwrap();
    let old_source = &original["installSource"];
    let managed = root.path().join("managed/cli");
    fs::create_dir_all(managed.join("versions")).unwrap();
    let state = managed.join("state.json");
    let sidecar = managed.join("versions/tool.market.json");
    fs::write(
        &state,
        serde_json::to_vec(&serde_json::json!({
            "installSource": old_source, "previousInstallSource": old_source,
            "activeVersion": "1.0.0", "checksum": "unchanged"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(&sidecar, serde_json::to_vec(old_source).unwrap()).unwrap();
    let output = startup_migrate(root.path());
    assert!(output.status.success(), "{output:?}");
    for path in [&install, &state, &sidecar] {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let source = if path == &sidecar {
            &value
        } else {
            &value["installSource"]
        };
        let _: local_app_contract::InstallSource =
            local_app_contract::decode(&serde_json::to_vec(source).unwrap()).unwrap();
        assert_eq!(source["source"], old_source["source"]);
        assert!(source.get("marketKey").is_none());
        assert!(path
            .with_extension("json.before-environment-identity-3")
            .is_file());
    }
    let before_retry = fs::read(&state).unwrap();
    assert!(startup_migrate(root.path()).status.success());
    assert_eq!(fs::read(&state).unwrap(), before_retry);
}

#[test]
fn fresh_startup_needs_no_config_or_application_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let output = startup_migrate(root.path());
    assert!(output.status.success(), "{output:?}");
    assert!(!root.path().join("config/agent-config.json").exists());
    assert!(!root.path().join("local-apps").exists());
}

#[test]
fn invalid_source_preflight_leaves_every_record_unchanged() {
    let root = fixture();
    let valid = root.path().join("local-apps/example/install.json");
    let before = fs::read(&valid).unwrap();
    let invalid = root.path().join("local-apps/invalid");
    fs::create_dir_all(&invalid).unwrap();
    fs::write(
        invalid.join("install.json"),
        br#"{"installSource":{"kind":"market"}}"#,
    )
    .unwrap();
    assert!(!startup_migrate(root.path()).status.success());
    assert_eq!(fs::read(&valid).unwrap(), before);
    assert!(!valid
        .with_extension("json.before-environment-identity-3")
        .exists());
}

#[test]
fn failed_owner_shutdown_blocks_migration_and_can_be_retried() {
    let root = fixture();
    let path = root.path().join("local-apps/example/install.json");
    let before = fs::read(&path).unwrap();
    let config = root.path().join("config/agent-config.json");
    fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({"local_apps": [{
            "appId": "example", "stopCommand": {
                "type": "shell_command", "command": ["nonexistent-migration-test-command"]
            }
        }]}))
        .unwrap(),
    )
    .unwrap();
    assert!(!startup_migrate(root.path()).status.success());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::write(&config, br#"{"local_apps":[]}"#).unwrap();
    assert!(startup_migrate(root.path()).status.success());
}

#[test]
#[ignore = "subprocess fixture used as an application-owned stop command"]
fn owner_stop_command() {
    fs::write(
        std::env::var_os("IDENTITY_MIGRATION_WRITER_STOP").unwrap(),
        b"stop",
    )
    .unwrap();
    // A real stop command must await its daemon's termination.
    std::thread::sleep(Duration::from_millis(200));
}

#[test]
fn startup_stops_leftover_089_application_before_converting_its_record() {
    let root = fixture();
    let installation = root.path().join("local-apps/example");
    let ready = installation.join("writer-ready");
    let stop = installation.join("writer-stop");
    let executable = installation.join(if cfg!(windows) {
        "writer.exe"
    } else {
        "writer"
    });
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let mut writer = Writer(
        Command::new(&executable)
            .args(["--exact", "installation_writer_process", "--ignored"])
            .current_dir(&installation)
            .env("IDENTITY_MIGRATION_WRITER_READY", &ready)
            .env("IDENTITY_MIGRATION_WRITER_STOP", &stop)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(writer.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(
        root.path().join("config/agent-config.json"),
        serde_json::to_vec(&serde_json::json!({"local_apps": [{
            "appId": "example", "stopCommand": {
                "type": "shell_command",
                "command": [std::env::current_exe().unwrap().to_str().unwrap(),
                    "--exact", "owner_stop_command", "--ignored"],
                "env": {"IDENTITY_MIGRATION_WRITER_STOP": stop.to_str().unwrap()}
            }
        }]}))
        .unwrap(),
    )
    .unwrap();
    let output = startup_migrate(root.path());
    assert!(output.status.success(), "{output:?}");
    assert!(writer.0.try_wait().unwrap().is_some());
    let record = fs::read_to_string(installation.join("install.json")).unwrap();
    assert!(!record.contains("marketKey"));
}
