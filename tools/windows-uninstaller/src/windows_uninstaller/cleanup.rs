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
