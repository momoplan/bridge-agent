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
    std::thread::sleep(Duration::from_secs(30));
}
