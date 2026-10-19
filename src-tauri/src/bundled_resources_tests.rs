use super::*;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "uses released native CLI bytes and an installed Linux package layout"]
fn linux_cli_packaged_resources_acceptance() {
    if std::env::var_os("BAIJIMU_RESOURCE_LAYOUT_CHILD").is_some() {
        verify_installed_resources();
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let context = tauri::generate_context!();
    let bin = fixture.path().join("usr/bin");
    let resources = fixture
        .path()
        .join("usr/lib")
        .join(&context.package_info().name);
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(resources.join("resources/bin")).unwrap();
    let executable = bin.join("desktop-resource-acceptance");
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let zip = PathBuf::from(std::env::var_os("BAIJIMU_CLI_UPGRADE_FIXTURE_DIR").unwrap())
        .join("current.zip");
    let mut archive = zip::ZipArchive::new(Cursor::new(fs::read(zip).unwrap())).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("bin/baijimu")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let cli = resources.join("resources/bin/baijimu");
    fs::write(&cli, bytes).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        resources.join("resources/bin/baijimu.market.json"),
        include_str!("../../tools/baijimu-cli/SOURCE.json"),
    )
    .unwrap();
    let cwd = fixture.path().join("unrelated-working-directory");
    fs::create_dir_all(&cwd).unwrap();
    let result = Command::new(&executable)
        .args([
            "--exact",
            "desktop::startup::packaged_resource_tests::linux_cli_packaged_resources_acceptance",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(cwd)
        .env("BAIJIMU_RESOURCE_LAYOUT_CHILD", &resources)
        .env("BAIJIMU_MANAGED_TOOL_ROOT", fixture.path().join("managed"))
        .env(
            "BAIJIMU_MANAGED_BIN_DIR",
            fixture.path().join("managed-bin"),
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("Linux installed package resource lookup and CLI bootstrap passed from an unrelated working directory");
}

fn verify_installed_resources() {
    let context = tauri::generate_context!();
    let root =
        tauri::utils::platform::resource_dir(context.package_info(), &Default::default()).unwrap();
    let expected = PathBuf::from(std::env::var_os("BAIJIMU_RESOURCE_LAYOUT_CHILD").unwrap());
    assert_eq!(root, expected);
    crate::bundled_resources::initialize(root).unwrap();
    let cli = bundled_baijimu_cli_path().unwrap();
    assert_eq!(cli, expected.join("resources/bin/baijimu"));
    let status = crate::managed_tool::bootstrap_bundled(Some(&cli)).unwrap();
    assert_eq!(
        status.installed_version.as_deref(),
        Some(include_str!("../../tools/baijimu-cli/VERSION").trim())
    );
    assert!(status.install_source.is_some());
    assert!(Path::new(&status.launcher_path).is_file());
}
