//! Operating system version the release service selects compatible releases by.
//! The update check and the Relay connection report the same value.

#[cfg(target_os = "macos")]
pub fn current_os_version() -> Result<Option<String>, String> {
    // sw_vers is available on Catalina. Do not depend on newer kernel sysctls.
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .env("SYSTEM_VERSION_COMPAT", "0")
        .output()
        .map_err(|err| format!("读取 macOS 版本失败: {err}"))?;
    if !output.status.success() {
        return Err(format!("读取 macOS 版本失败: {}", output.status));
    }
    let version = std::str::from_utf8(&output.stdout)
        .map_err(|err| format!("macOS 版本编码无效: {err}"))?
        .trim();
    let parts: Vec<_> = version.split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("macOS 未返回有效的系统版本".to_string());
    }
    Ok(Some(version.to_string()))
}

#[cfg(not(target_os = "macos"))]
pub fn current_os_version() -> Result<Option<String>, String> {
    Ok(None)
}
