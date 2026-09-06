#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("bridge-agent-uninstaller is only supported on Windows");
}

#[cfg(windows)]
mod windows_uninstaller {
    use anyhow::{bail, Context, Result};
    use std::ffi::OsStr;
    use std::fs;
    use std::os::windows::process::CommandExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitStatus};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, SendMessageTimeoutW, HWND_BROADCAST, IDCANCEL, IDNO, IDYES, MB_ICONERROR,
        MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNOCANCEL, SMTO_ABORTIFHUNG,
        WM_SETTINGCHANGE,
    };
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;
    use winreg::{RegKey, RegValue};

    const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const PRODUCT_REGISTRY_KEY: &str = r"Software\Baijimu\BridgeAgent";
    const PRODUCT_CODE_VALUE: &str = "ProductCode";
    const AUTOSTART_REGISTRY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const AUTOSTART_VALUE: &str = "BaijimuBridgeAgent";
    const MANAGED_CLI_ID: &str = "baijimu-cli";
    const APP_IDENTIFIER: &str = "com.baijimu.bridgeagent";
    const DESKTOP_EXECUTABLE: &str = "bridge-agent-desktop.exe";
    const UNINSTALLER_EXECUTABLE: &str = "bridge-agent-uninstaller.exe";
    const FULL_UNINSTALL_PROPERTY: &str = "BAIJIMU_REMOVE_USER_DATA=1";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CleanupMode {
        PreserveUserData,
        RemoveUserData,
    }

    #[derive(Debug, Default)]
    struct CleanupReport {
        errors: Vec<String>,
        notes: Vec<String>,
    }

    impl CleanupReport {
        fn error(&mut self, context: impl Into<String>, error: impl std::fmt::Display) {
            self.errors.push(format!("{}: {error}", context.into()));
        }

        fn note(&mut self, note: impl Into<String>) {
            self.notes.push(note.into());
        }

        fn finish(self) -> Result<()> {
            write_cleanup_log(&self);
            if self.errors.is_empty() {
                Ok(())
            } else {
                bail!(self.errors.join("\n"))
            }
        }
    }

    pub fn run() -> Result<()> {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        if has_arg(&args, "--msi-cleanup") {
            let mode = if value_after(&args, "--msi-cleanup").as_deref() == Some("1") {
                CleanupMode::RemoveUserData
            } else {
                CleanupMode::PreserveUserData
            };
            return cleanup(mode);
        }

        if has_arg(&args, "--worker") {
            return run_worker(&args);
        }

        launch_temporary_worker(&args)
    }

    fn launch_temporary_worker(args: &[std::ffi::OsString]) -> Result<()> {
        let current = std::env::current_exe().context("无法确定卸载器路径")?;
        let temporary_dir = std::env::temp_dir().join("Baijimu").join("uninstaller");
        fs::create_dir_all(&temporary_dir).context("无法创建卸载器临时目录")?;
        let temporary = temporary_dir.join(format!(
            "bridge-agent-uninstaller-{}-{}.exe",
            std::process::id(),
            now_ms()
        ));
        fs::copy(&current, &temporary)
            .with_context(|| format!("无法把卸载器复制到临时目录 {}", temporary.display()))?;

        let mut command = Command::new(&temporary);
        command.arg("--worker");
        for arg in args {
            if arg != OsStr::new("--interactive") {
                command.arg(arg);
            }
        }
        configure_hidden(&mut command);
        command.spawn().context("无法启动百积木卸载向导")?;
        Ok(())
    }

    fn run_worker(args: &[std::ffi::OsString]) -> Result<()> {
        let quiet = has_arg(args, "--quiet");
        let mode = if has_arg(args, "--full") {
            CleanupMode::RemoveUserData
        } else if has_arg(args, "--preserve-data") {
            CleanupMode::PreserveUserData
        } else {
            match ask_cleanup_mode() {
                Some(mode) => mode,
                None => return Ok(()),
            }
        };
        let product_code = value_after(args, "--product-code")
            .map(Ok)
            .unwrap_or_else(read_product_code)?;

        let mut command = Command::new("msiexec.exe");
        command.args(["/x", &product_code, "/norestart"]);
        if mode == CleanupMode::RemoveUserData {
            command.arg(FULL_UNINSTALL_PROPERTY);
        }
        if quiet {
            command.arg("/qn");
        }
        let status = command.status().context("无法启动 Windows Installer")?;
        schedule_self_removal();

        if msi_succeeded(status) {
            if !quiet {
                show_message(
                    "百积木卸载完成",
                    if mode == CleanupMode::RemoveUserData {
                        "百积木及其本机数据已经完全卸载。"
                    } else {
                        "百积木已经卸载，设备配置和本地应用数据已保留，重新安装后可继续使用。"
                    },
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            Ok(())
        } else {
            let code = status.code().unwrap_or(-1);
            if !quiet {
                show_message(
                    "百积木卸载失败",
                    &format!("Windows Installer 返回错误码 {code}，请查看系统安装日志后重试。"),
                    MB_OK | MB_ICONERROR,
                );
            }
            bail!("Windows Installer returned exit code {code}")
        }
    }

    fn ask_cleanup_mode() -> Option<CleanupMode> {
        let result = show_message(
            "卸载百积木",
            "请选择卸载方式：\n\n“是”——完全卸载，同时删除设备配置、登录凭证、本地应用、Baijimu CLI 和相关数据。\n\n“否”——只卸载客户端，保留上述数据，便于以后重新安装。\n\n“取消”——不执行卸载。",
            MB_YESNOCANCEL | MB_ICONQUESTION,
        );
        match result {
            IDYES => Some(CleanupMode::RemoveUserData),
            IDNO => Some(CleanupMode::PreserveUserData),
            IDCANCEL => None,
            _ => None,
        }
    }

    fn cleanup(mode: CleanupMode) -> Result<()> {
        let mut report = CleanupReport::default();
        stop_desktop(&mut report);
        terminate_connector_package_processes(&mut report);
        remove_autostart(&mut report);
        remove_transient_state(&mut report);
        if mode == CleanupMode::RemoveUserData {
            remove_managed_cli(&mut report);
            remove_full_user_data(&mut report);
        }
        report.finish()
    }

    fn stop_desktop(report: &mut CleanupReport) {
        if !desktop_is_running() {
            return;
        }
        if let Some(desktop) = installed_desktop_path() {
            let mut command = Command::new(&desktop);
            command.arg("--quit-running-instance");
            configure_hidden(&mut command);
            if let Err(error) = command.status() {
                report.note(format!("请求桌面端正常退出失败，将强制终止：{error}"));
            }
            if wait_for_desktop_exit(Duration::from_secs(20)) {
                return;
            }
        }

        let mut command = Command::new("taskkill.exe");
        command.args(["/F", "/T", "/IM", DESKTOP_EXECUTABLE]);
        configure_hidden(&mut command);
        match command.status() {
            Ok(_) if wait_for_desktop_exit(Duration::from_secs(10)) => {}
            Ok(status) => report.error(
                "无法终止仍在运行的百积木桌面端",
                format!("taskkill exit code {:?}", status.code()),
            ),
            Err(error) => report.error("无法启动 taskkill", error),
        }
    }

    fn terminate_connector_package_processes(report: &mut CleanupReport) {
        let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) else {
            return;
        };
        let connectors = app_data
            .join("baijimu")
            .join("bridge-agent")
            .join("config")
            .join("connectors");
        if !connectors.is_dir() {
            return;
        }
        let escaped = connectors.to_string_lossy().replace('\'', "''");
        let mut script =
            format!("$root = [IO.Path]::GetFullPath('{escaped}').TrimEnd('\\') + '\\'; ");
        script.push_str("Get-CimInstance Win32_Process | Where-Object { ($_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) -or ($_.CommandLine -and $_.CommandLine.IndexOf($root, [StringComparison]::OrdinalIgnoreCase) -ge 0) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop }");
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ]);
        configure_hidden(&mut command);
        match command.status() {
            Ok(status) if status.success() => {}
            Ok(status) => report.error(
                "终止 Connector 安装目录中的遗留进程",
                format!("PowerShell exit code {:?}", status.code()),
            ),
            Err(error) => report.error("启动 Connector 进程清理", error),
        }
    }

    fn remove_autostart(report: &mut CleanupReport) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey_with_flags(AUTOSTART_REGISTRY_KEY, winreg::enums::KEY_WRITE) {
            Ok(key) => {
                if let Err(error) = key.delete_value(AUTOSTART_VALUE) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        report.error("删除登录启动项", error);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => report.error("打开登录启动注册表", error),
        }
    }

    fn remove_transient_state(report: &mut CleanupReport) {
        if let Some(program_data) = program_data_dir() {
            for name in [
                "bridge-agent-desktop-startup.log",
                "bridge-agent-desktop-startup-state.json",
                "bridge-agent-desktop-interactive-restart",
                "local-app-control.json",
            ] {
                remove_file(&program_data.join(name), report);
            }
            remove_dir(&program_data.join(".bridge-agent-locks"), report);
            remove_dir(&program_data.join("logs"), report);
        }
        if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
            remove_dir(&app_data.join(APP_IDENTIFIER), report);
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            remove_dir(&local_app_data.join(APP_IDENTIFIER), report);
            remove_dir(&local_app_data.join("baijimu").join("bridge-agent"), report);
        }
        remove_file(
            &std::env::temp_dir().join("bridge-agent-desktop-startup.log"),
            report,
        );
    }

    fn remove_managed_cli(report: &mut CleanupReport) {
        let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
            report.error("删除 Baijimu CLI", "LOCALAPPDATA 不可用");
            return;
        };
        let cli_root = local_app_data
            .join("Baijimu")
            .join("apps")
            .join(MANAGED_CLI_ID);
        let bin_dir = local_app_data.join("Baijimu").join("bin");
        remove_dir(&cli_root, report);
        remove_file(&bin_dir.join("baijimu.exe"), report);
        remove_windows_user_path_entry(&bin_dir, report);
        remove_dir_if_empty(&bin_dir);
        remove_dir_if_empty(&local_app_data.join("Baijimu").join("apps"));
        remove_dir_if_empty(&local_app_data.join("Baijimu"));
    }

    fn remove_full_user_data(report: &mut CleanupReport) {
        if let Some(program_data) = program_data_dir() {
            remove_dir(&program_data, report);
            if let Some(parent) = program_data.parent() {
                remove_dir_if_empty(parent);
            }
        }
        if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
            remove_dir(&app_data.join("baijimu").join("bridge-agent"), report);
            remove_dir_if_empty(&app_data.join("baijimu"));
        }
        if let Some(home) = dirs::home_dir() {
            remove_file(
                &home.join(".config").join("baijimu").join("auth.json"),
                report,
            );
            remove_dir_if_empty(&home.join(".config").join("baijimu"));
            remove_dir_if_empty(&home.join(".config"));
            remove_dir(
                &home.join(".agents").join("skills").join("baijimu-platform"),
                report,
            );
            remove_dir_if_empty(&home.join(".agents").join("skills"));
        }
    }

    include!("windows_uninstaller/cleanup.rs");
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_uninstaller::run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
