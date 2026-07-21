use anyhow::{Context, Result};
#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
use sha2::{Digest, Sha256};
#[cfg(any(test, windows))]
use std::fs;
use std::path::Path;
#[cfg(any(test, windows))]
use std::path::PathBuf;

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
const KEYRING_SERVICE: &str = "com.baijimu.bridge-agent.relay";
#[cfg(all(not(test), windows))]
const WINDOWS_SECRET_SUFFIX: &str = "credentials";

pub fn load_relay_token(config_path: &Path) -> Result<Option<String>> {
    load_secret(config_path).map(|value| {
        value.and_then(|value| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        })
    })
}

pub fn store_relay_token(config_path: &Path, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        return delete_relay_token(config_path);
    }
    store_secret(config_path, token)
}

pub fn delete_relay_token(config_path: &Path) -> Result<()> {
    delete_secret(config_path)
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
fn credential_id(config_path: &Path) -> String {
    let normalized = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf())
        .to_string_lossy()
        .to_string();
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

#[cfg(test)]
fn test_secret_path(config_path: &Path) -> PathBuf {
    config_path.with_extension("test-credentials")
}

#[cfg(test)]
fn load_secret(config_path: &Path) -> Result<Option<String>> {
    let path = test_secret_path(config_path);
    match fs::read_to_string(&path) {
        Ok(value) => Ok(Some(value)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

#[cfg(test)]
fn store_secret(config_path: &Path, value: &str) -> Result<()> {
    let path = test_secret_path(config_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, value)?;
    Ok(())
}

#[cfg(test)]
fn delete_secret(config_path: &Path) -> Result<()> {
    let path = test_secret_path(config_path);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
fn keyring_entry(config_path: &Path) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, &credential_id(config_path))
        .context("failed to open the operating system credential store")
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
fn load_secret(config_path: &Path) -> Result<Option<String>> {
    let entry = keyring_entry(config_path)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err)
            .context("failed to read relay token from the operating system credential store"),
    }
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
fn store_secret(config_path: &Path, value: &str) -> Result<()> {
    keyring_entry(config_path)?
        .set_password(value)
        .context("failed to store relay token in the operating system credential store")
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "linux")))]
fn delete_secret(config_path: &Path) -> Result<()> {
    let entry = keyring_entry(config_path)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err)
            .context("failed to delete relay token from the operating system credential store"),
    }
}

#[cfg(all(not(test), windows))]
fn windows_secret_path(config_path: &Path) -> PathBuf {
    config_path.with_extension(WINDOWS_SECRET_SUFFIX)
}

#[cfg(all(not(test), windows))]
fn load_secret(config_path: &Path) -> Result<Option<String>> {
    let path = windows_secret_path(config_path);
    let encrypted = match fs::read(&path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    decrypt_windows_secret(&encrypted).map(Some)
}

#[cfg(all(not(test), windows))]
fn store_secret(config_path: &Path, value: &str) -> Result<()> {
    let path = windows_secret_path(config_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let protected = encrypt_windows_secret(value.as_bytes())?;
    let temporary = path.with_extension(format!("{WINDOWS_SECRET_SUFFIX}.tmp"));
    fs::write(&temporary, protected)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to replace {}", path.display()))?;
    }
    fs::rename(&temporary, &path)
        .with_context(|| format!("failed to commit {}", path.display()))?;
    Ok(())
}

#[cfg(all(not(test), windows))]
fn delete_secret(config_path: &Path) -> Result<()> {
    let path = windows_secret_path(config_path);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to delete {}", path.display())),
    }
}

#[cfg(all(not(test), windows))]
fn encrypt_windows_secret(value: &[u8]) -> Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        NCryptCloseProtectionDescriptor, NCryptCreateProtectionDescriptor, NCryptProtectSecret,
        NCRYPT_SILENT_FLAG,
    };
    use windows_sys::Win32::Security::NCRYPT_DESCRIPTOR_HANDLE;

    let descriptor_text = "LOCAL=machine\0".encode_utf16().collect::<Vec<_>>();
    let mut descriptor: NCRYPT_DESCRIPTOR_HANDLE = null_mut();
    let status =
        unsafe { NCryptCreateProtectionDescriptor(descriptor_text.as_ptr(), 0, &mut descriptor) };
    hresult(status, "create Windows protection descriptor")?;

    let mut protected = null_mut();
    let mut protected_len = 0_u32;
    let status = unsafe {
        NCryptProtectSecret(
            descriptor,
            NCRYPT_SILENT_FLAG,
            value.as_ptr(),
            u32::try_from(value.len()).context("relay token is too large")?,
            null(),
            null_mut(),
            &mut protected,
            &mut protected_len,
        )
    };
    unsafe { NCryptCloseProtectionDescriptor(descriptor) };
    hresult(status, "protect relay token with Windows CNG DPAPI")?;
    let bytes = unsafe { std::slice::from_raw_parts(protected, protected_len as usize) }.to_vec();
    unsafe { LocalFree(protected.cast()) };
    Ok(bytes)
}

#[cfg(all(not(test), windows))]
fn decrypt_windows_secret(value: &[u8]) -> Result<String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        NCryptCloseProtectionDescriptor, NCryptUnprotectSecret, NCRYPT_SILENT_FLAG,
    };
    use windows_sys::Win32::Security::NCRYPT_DESCRIPTOR_HANDLE;

    let mut descriptor: NCRYPT_DESCRIPTOR_HANDLE = null_mut();
    let mut plaintext = null_mut();
    let mut plaintext_len = 0_u32;
    let status = unsafe {
        NCryptUnprotectSecret(
            &mut descriptor,
            NCRYPT_SILENT_FLAG,
            value.as_ptr(),
            u32::try_from(value.len()).context("protected relay token is too large")?,
            null(),
            null_mut(),
            &mut plaintext,
            &mut plaintext_len,
        )
    };
    hresult(status, "unprotect relay token with Windows CNG DPAPI")?;
    let bytes = unsafe { std::slice::from_raw_parts(plaintext, plaintext_len as usize) }.to_vec();
    unsafe {
        if !descriptor.is_null() {
            NCryptCloseProtectionDescriptor(descriptor);
        }
        LocalFree(plaintext.cast());
    }
    String::from_utf8(bytes).context("protected relay token is not valid UTF-8")
}

#[cfg(all(not(test), windows))]
fn hresult(status: i32, action: &str) -> Result<()> {
    if status >= 0 {
        Ok(())
    } else {
        anyhow::bail!("failed to {action}: HRESULT 0x{:08X}", status as u32)
    }
}

#[cfg(all(not(test), not(any(target_os = "macos", target_os = "linux", windows))))]
compile_error!("bridge-agent secure credential storage is unsupported on this platform");

#[cfg(test)]
mod tests {
    use super::{delete_relay_token, load_relay_token, store_relay_token};

    #[test]
    fn test_store_round_trip_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        assert!(load_relay_token(&config_path).unwrap().is_none());
        store_relay_token(&config_path, "secret-token").unwrap();
        assert_eq!(
            load_relay_token(&config_path).unwrap().as_deref(),
            Some("secret-token")
        );
        delete_relay_token(&config_path).unwrap();
        assert!(load_relay_token(&config_path).unwrap().is_none());
    }
}
