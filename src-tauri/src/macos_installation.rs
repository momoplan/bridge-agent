#[cfg(any(all(target_os = "macos", not(debug_assertions)), test))]
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[cfg(target_os = "macos")]
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

#[cfg(any(all(target_os = "macos", not(debug_assertions)), test))]
fn app_bundle_from_executable(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|path| path.extension() == Some(OsStr::new("app")))
        .map(Path::to_path_buf)
}

#[cfg(any(all(target_os = "macos", not(debug_assertions)), test))]
fn executable_requires_installation(executable: &Path, home_dir: Option<&Path>) -> bool {
    let Some(app_bundle) = app_bundle_from_executable(executable) else {
        return false;
    };

    let installed_in_system_applications = [
        Path::new("/Applications"),
        Path::new("/System/Applications"),
    ]
    .iter()
    .any(|root| app_bundle.starts_with(root));
    let installed_in_user_applications = home_dir
        .map(|home| app_bundle.starts_with(home.join("Applications")))
        .unwrap_or(false);

    !installed_in_system_applications && !installed_in_user_applications
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
pub fn required_for_current_executable() -> bool {
    std::env::current_exe().ok().is_some_and(|executable| {
        executable_requires_installation(&executable, dirs::home_dir().as_deref())
    })
}

#[cfg(any(not(target_os = "macos"), debug_assertions))]
pub fn required_for_current_executable() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn show_reminder(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    app.dialog()
        .message(
            "请退出百积木，将“百积木.app”拖到“应用程序”文件夹，然后从“应用程序”中重新打开。\n\n安装后运行，才能保证自动更新、开机启动和系统权限稳定生效。",
        )
        .title("请先安装百积木")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCustom("退出并安装".to_string()))
        .show(move |_| app_handle.exit(0));
}

#[cfg(not(target_os = "macos"))]
pub fn show_reminder(_app: &tauri::AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_apps_outside_applications_directories() {
        let home = Path::new("/Users/tester");
        assert!(executable_requires_installation(
            Path::new("/Volumes/百积木/百积木.app/Contents/MacOS/bridge-agent-desktop"),
            Some(home),
        ));
        assert!(executable_requires_installation(
            Path::new("/private/var/folders/x/AppTranslocation/d/百积木.app/Contents/MacOS/bridge-agent-desktop"),
            Some(home),
        ));
        assert!(executable_requires_installation(
            Path::new("/Users/tester/Downloads/百积木.app/Contents/MacOS/bridge-agent-desktop"),
            Some(home),
        ));
    }

    #[test]
    fn accepts_system_and_user_applications_directories() {
        let home = Path::new("/Users/tester");
        assert!(!executable_requires_installation(
            Path::new("/Applications/百积木.app/Contents/MacOS/bridge-agent-desktop"),
            Some(home),
        ));
        assert!(!executable_requires_installation(
            Path::new("/Applications/Business/百积木.app/Contents/MacOS/bridge-agent-desktop"),
            Some(home),
        ));
        assert!(!executable_requires_installation(
            Path::new("/Users/tester/Applications/百积木.app/Contents/MacOS/bridge-agent-desktop"),
            Some(home),
        ));
        assert!(!executable_requires_installation(
            Path::new("/tmp/bridge-agent-desktop"),
            Some(home),
        ));
    }
}
