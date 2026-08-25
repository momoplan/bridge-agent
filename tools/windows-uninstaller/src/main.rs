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
    const MANAGED_CLI_ID: &str = "com.baijimu.cli";
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

    fn remove_windows_user_path_entry(path: &Path, report: &mut CleanupReport) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let environment = match hkcu.open_subkey_with_flags(
            "Environment",
            winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
        ) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                report.error("打开用户 PATH", error);
                return;
            }
        };
        let existing_raw = match environment.get_raw_value("Path") {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                report.error("读取用户 PATH", error);
                return;
            }
        };
        let existing = match String::from_reg_value(&existing_raw) {
            Ok(value) => value,
            Err(error) => {
                report.error("解析用户 PATH", error);
                return;
            }
        };
        let updated = remove_path_entry(&existing, path);
        if updated == existing {
            return;
        }
        let value_type = if existing_raw.vtype == REG_SZ || existing_raw.vtype == REG_EXPAND_SZ {
            existing_raw.vtype
        } else {
            REG_EXPAND_SZ
        };
        let value = RegValue {
            bytes: updated
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
            vtype: value_type,
        };
        if let Err(error) = environment.set_raw_value("Path", &value) {
            report.error("更新用户 PATH", error);
            return;
        }
        broadcast_environment_change();
    }

    fn remove_path_entry(existing: &str, target: &Path) -> String {
        let normalized_target = normalize_windows_path(&target.to_string_lossy());
        existing
            .split(';')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .filter(|entry| normalize_windows_path(entry) != normalized_target)
            .collect::<Vec<_>>()
            .join(";")
    }

    fn normalize_windows_path(value: &str) -> String {
        let expanded = expand_windows_environment(value);
        expanded
            .trim()
            .trim_matches('"')
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_ascii_lowercase()
    }

    fn expand_windows_environment(value: &str) -> String {
        let mut result = value.to_string();
        for name in ["LOCALAPPDATA", "APPDATA", "USERPROFILE"] {
            let marker = format!("%{name}%");
            if let Some(replacement) = std::env::var_os(name) {
                result = replace_ascii_case_insensitive(
                    &result,
                    &marker,
                    &replacement.to_string_lossy(),
                );
            }
        }
        result
    }

    fn replace_ascii_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
        let lower_value = value.to_ascii_lowercase();
        let lower_needle = needle.to_ascii_lowercase();
        let mut result = String::new();
        let mut start = 0;
        while let Some(relative) = lower_value[start..].find(&lower_needle) {
            let index = start + relative;
            result.push_str(&value[start..index]);
            result.push_str(replacement);
            start = index + needle.len();
        }
        result.push_str(&value[start..]);
        result
    }

    fn broadcast_environment_change() {
        let environment = wide("Environment");
        unsafe {
            let mut result = 0usize;
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM::default(),
                environment.as_ptr() as LPARAM,
                SMTO_ABORTIFHUNG,
                5_000,
                &mut result,
            );
        }
    }

    fn installed_desktop_path() -> Option<PathBuf> {
        let current = std::env::current_exe().ok()?;
        current
            .parent()
            .map(|parent| parent.join(DESKTOP_EXECUTABLE))
    }

    fn desktop_is_running() -> bool {
        let mut command = Command::new("tasklist.exe");
        command.args([
            "/FI",
            "IMAGENAME eq bridge-agent-desktop.exe",
            "/FO",
            "CSV",
            "/NH",
        ]);
        configure_hidden(&mut command);
        command.output().is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains("bridge-agent-desktop.exe")
        })
    }

    fn wait_for_desktop_exit(timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !desktop_is_running() {
                return true;
            }
            thread::sleep(Duration::from_millis(250));
        }
        !desktop_is_running()
    }

    fn read_product_code() -> Result<String> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm
            .open_subkey(PRODUCT_REGISTRY_KEY)
            .context("找不到百积木安装信息，请从 Windows“已安装的应用”中卸载")?;
        let code: String = key
            .get_value(PRODUCT_CODE_VALUE)
            .context("百积木安装信息缺少 ProductCode")?;
        if !looks_like_product_code(&code) {
            bail!("百积木 ProductCode 格式无效")
        }
        Ok(code)
    }

    fn looks_like_product_code(value: &str) -> bool {
        value.len() == 38
            && value.starts_with('{')
            && value.ends_with('}')
            && value
                .chars()
                .skip(1)
                .take(36)
                .enumerate()
                .all(|(index, ch)| match index {
                    8 | 13 | 18 | 23 => ch == '-',
                    _ => ch.is_ascii_hexdigit(),
                })
    }

    fn msi_succeeded(status: ExitStatus) -> bool {
        matches!(status.code(), Some(0 | 1605 | 3010))
    }

    fn program_data_dir() -> Option<PathBuf> {
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .map(|path| path.join("Baijimu").join("BridgeAgent"))
    }

    fn remove_file(path: &Path, report: &mut CleanupReport) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => report.error(format!("删除文件 {}", path.display()), error),
        }
    }

    fn remove_dir(path: &Path, report: &mut CleanupReport) {
        match fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => report.error(format!("删除目录 {}", path.display()), error),
        }
    }

    fn remove_dir_if_empty(path: &Path) {
        if fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(path);
        }
    }

    fn write_cleanup_log(report: &CleanupReport) {
        let mut lines = vec![format!("{} bridge-agent uninstall cleanup", now_ms())];
        lines.extend(report.notes.iter().map(|line| format!("NOTE {line}")));
        lines.extend(report.errors.iter().map(|line| format!("ERROR {line}")));
        if report.notes.is_empty() && report.errors.is_empty() {
            lines.push("OK cleanup completed".to_string());
        }
        let _ = fs::write(
            std::env::temp_dir().join("baijimu-uninstall.log"),
            format!("{}\r\n", lines.join("\r\n")),
        );
    }

    fn schedule_self_removal() {
        let Ok(current) = std::env::current_exe() else {
            return;
        };
        if current.file_name() == Some(OsStr::new(UNINSTALLER_EXECUTABLE)) {
            return;
        }
        let escaped = current.to_string_lossy().replace('\'', "''");
        let script = format!(
            "Start-Sleep -Milliseconds 750; Remove-Item -LiteralPath '{escaped}' -Force -ErrorAction SilentlyContinue"
        );
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
        let _ = command.spawn();
    }

    fn configure_hidden(command: &mut Command) {
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    }

    fn has_arg(args: &[std::ffi::OsString], name: &str) -> bool {
        args.iter().any(|arg| arg == OsStr::new(name))
    }

    fn value_after(args: &[std::ffi::OsString], name: &str) -> Option<String> {
        args.iter()
            .position(|arg| arg == OsStr::new(name))
            .and_then(|index| args.get(index + 1))
            .and_then(|value| value.to_str())
            .map(str::to_string)
    }

    fn show_message(title: &str, message: &str, flags: u32) -> i32 {
        let title = wide(title);
        let message = wide(message);
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                flags,
            )
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn now_ms() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn product_code_validation_is_strict() {
            assert!(looks_like_product_code(
                "{94895101-CD67-53B8-BB30-F95026802DF2}"
            ));
            assert!(!looks_like_product_code(
                "94895101-CD67-53B8-BB30-F95026802DF2"
            ));
            assert!(!looks_like_product_code(
                "{94895101-CD67-53B8-BB30-F95026802DFX}"
            ));
        }

        #[test]
        fn path_cleanup_removes_only_the_managed_bin_entry() {
            std::env::set_var("LOCALAPPDATA", r"C:\Users\Ada\AppData\Local");
            let updated = remove_path_entry(
                r"C:\Tools;%LOCALAPPDATA%\Baijimu\bin;C:\Windows\System32;C:\Tools2",
                Path::new(r"C:\Users\Ada\AppData\Local\Baijimu\bin"),
            );
            assert_eq!(updated, r"C:\Tools;C:\Windows\System32;C:\Tools2");
        }

        #[test]
        fn path_cleanup_preserves_similar_and_unrelated_entries() {
            std::env::set_var("LOCALAPPDATA", r"C:\Users\Ada\AppData\Local");
            let existing = r"C:\Baijimu\bin-extra;C:\Tools";
            assert_eq!(
                remove_path_entry(
                    existing,
                    Path::new(r"C:\Users\Ada\AppData\Local\Baijimu\bin")
                ),
                existing
            );
        }
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_uninstaller::run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
