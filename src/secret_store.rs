use anyhow::{Context, Result};
#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
use sha2::{Digest, Sha256};
#[cfg(any(test, windows))]
use std::fs;
use std::path::Path;
#[cfg(any(
    test,
    windows,
    all(debug_assertions, any(target_os = "macos", target_os = "linux"))
))]
use std::path::PathBuf;

#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
const KEYRING_SERVICE: &str = "com.baijimu.bridge-agent.relay";
#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
const KEYRING_SERVICE_REGISTRATION: &str = "com.baijimu.bridge-agent.service-registration";
#[cfg(any(
    test,
    all(debug_assertions, any(target_os = "macos", target_os = "linux"))
))]
const DEVELOPMENT_SECRET_DIRECTORY: &str = ".bridge-agent-development";
#[cfg(all(not(test), windows))]
const WINDOWS_SECRET_SUFFIX: &str = "credentials";

#[derive(Clone, Copy)]
enum SecretSlot {
    Relay,
    ServiceRegistration,
}

fn secret_suffix(slot: SecretSlot) -> &'static str {
    match slot {
        SecretSlot::Relay => "credentials",
        SecretSlot::ServiceRegistration => "service-registration",
    }
}

pub fn load_relay_token(config_path: &Path) -> Result<Option<String>> {
    load_named_secret(config_path, SecretSlot::Relay)
}

pub fn load_service_registration_token(config_path: &Path) -> Result<Option<String>> {
    load_named_secret(config_path, SecretSlot::ServiceRegistration)
}

fn load_named_secret(config_path: &Path, slot: SecretSlot) -> Result<Option<String>> {
    load_secret(config_path, slot).map(|value| {
        value.and_then(|value| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        })
    })
}

/// Permission repair belongs to initialization, never to credential reads.
pub(crate) fn prepare_relay_credentials(config_path: &Path) -> Result<()> {
    #[cfg(any(
        all(test, unix),
        all(
            not(test),
            debug_assertions,
            any(target_os = "macos", target_os = "linux")
        )
    ))]
    {
        use std::os::unix::fs::PermissionsExt;
        for slot in [SecretSlot::Relay, SecretSlot::ServiceRegistration] {
            let path = development_secret_path(config_path, slot);
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.permissions().mode() & 0o077 != 0 => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    let _ = config_path;
    Ok(())
}

pub fn store_relay_token(config_path: &Path, token: &str) -> Result<()> {
    store_named_secret(config_path, SecretSlot::Relay, token)
}

pub fn store_service_registration_token(config_path: &Path, token: &str) -> Result<()> {
    store_named_secret(config_path, SecretSlot::ServiceRegistration, token)
}

fn store_named_secret(config_path: &Path, slot: SecretSlot, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        return delete_secret(config_path, slot);
    }
    store_secret(config_path, slot, token)
}

pub fn delete_relay_token(config_path: &Path) -> Result<()> {
    delete_secret(config_path, SecretSlot::Relay)
}

pub fn delete_service_registration_token(config_path: &Path) -> Result<()> {
    delete_secret(config_path, SecretSlot::ServiceRegistration)
}

#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
fn credential_id(config_path: &Path) -> String {
    let normalized = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf())
        .to_string_lossy()
        .to_string();
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

#[cfg(any(
    test,
    all(debug_assertions, any(target_os = "macos", target_os = "linux"))
))]
fn development_secret_path(config_path: &Path, slot: SecretSlot) -> PathBuf {
    let file_name = config_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "agent-config.development.json".into());
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(DEVELOPMENT_SECRET_DIRECTORY)
        .join(format!("{file_name}.{}", secret_suffix(slot)))
}

#[cfg(all(
    not(test),
    debug_assertions,
    any(target_os = "macos", target_os = "linux")
))]
fn load_secret(config_path: &Path, slot: SecretSlot) -> Result<Option<String>> {
    load_development_secret(config_path, slot)
}

#[cfg(all(
    not(test),
    debug_assertions,
    any(target_os = "macos", target_os = "linux")
))]
fn store_secret(config_path: &Path, slot: SecretSlot, value: &str) -> Result<()> {
    store_development_secret(config_path, slot, value)
}

#[cfg(all(
    not(test),
    debug_assertions,
    any(target_os = "macos", target_os = "linux")
))]
fn delete_secret(config_path: &Path, slot: SecretSlot) -> Result<()> {
    delete_development_secret(config_path, slot)
}

#[cfg(any(
    all(test, unix),
    all(
        not(test),
        debug_assertions,
        any(target_os = "macos", target_os = "linux")
    )
))]
fn load_development_secret(config_path: &Path, slot: SecretSlot) -> Result<Option<String>> {
    use std::os::unix::fs::PermissionsExt;

    let path = development_secret_path(config_path, slot);
    match std::fs::read_to_string(&path) {
        Ok(value) => {
            anyhow::ensure!(
                std::fs::metadata(&path)?.permissions().mode() & 0o077 == 0,
                "credential file permissions are not private; run configuration initialization"
            );
            Ok(Some(value))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

#[cfg(any(
    all(test, unix),
    all(
        not(test),
        debug_assertions,
        any(target_os = "macos", target_os = "linux")
    )
))]
fn store_development_secret(config_path: &Path, slot: SecretSlot, value: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let path = development_secret_path(config_path, slot);
    let parent = path
        .parent()
        .context("development credential path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", parent.display()))?;

    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "credentials".into()),
        uuid::Uuid::new_v4().simple()
    ));
    let write_result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        file.write_all(value.as_bytes())
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temporary.display()))?;
        std::fs::rename(&temporary, &path).with_context(|| {
            format!(
                "failed to commit development credentials from {} to {}",
                temporary.display(),
                path.display()
            )
        })?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", path.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result
}

#[cfg(any(
    all(test, unix),
    all(
        not(test),
        debug_assertions,
        any(target_os = "macos", target_os = "linux")
    )
))]
fn delete_development_secret(config_path: &Path, slot: SecretSlot) -> Result<()> {
    let path = development_secret_path(config_path, slot);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to delete {}", path.display())),
    }
}

#[cfg(test)]
fn test_secret_path(config_path: &Path, slot: SecretSlot) -> PathBuf {
    config_path.with_extension(format!("test-{}", secret_suffix(slot)))
}

#[cfg(test)]
fn load_secret(config_path: &Path, slot: SecretSlot) -> Result<Option<String>> {
    let path = test_secret_path(config_path, slot);
    match fs::read_to_string(&path) {
        Ok(value) => Ok(Some(value)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

#[cfg(test)]
fn store_secret(config_path: &Path, slot: SecretSlot, value: &str) -> Result<()> {
    let path = test_secret_path(config_path, slot);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, value)?;
    Ok(())
}

#[cfg(test)]
fn delete_secret(config_path: &Path, slot: SecretSlot) -> Result<()> {
    let path = test_secret_path(config_path, slot);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
fn keyring_entry(config_path: &Path, slot: SecretSlot) -> Result<keyring::Entry> {
    let service = match slot {
        SecretSlot::Relay => KEYRING_SERVICE,
        SecretSlot::ServiceRegistration => KEYRING_SERVICE_REGISTRATION,
    };
    keyring::Entry::new(service, &credential_id(config_path))
        .context("failed to open the operating system credential store")
}

#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
fn load_secret(config_path: &Path, slot: SecretSlot) -> Result<Option<String>> {
    let entry = keyring_entry(config_path, slot)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err)
            .context("failed to read relay token from the operating system credential store"),
    }
}

#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
fn store_secret(config_path: &Path, slot: SecretSlot, value: &str) -> Result<()> {
    let entry = keyring_entry(config_path, slot)?;
    if entry
        .get_password()
        .is_ok_and(|current_value| current_value == value)
    {
        return Ok(());
    }
    entry
        .set_password(value)
        .context("failed to store relay token in the operating system credential store")
}

#[cfg(all(
    not(test),
    not(debug_assertions),
    any(target_os = "macos", target_os = "linux")
))]
fn delete_secret(config_path: &Path, slot: SecretSlot) -> Result<()> {
    let entry = keyring_entry(config_path, slot)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err)
            .context("failed to delete relay token from the operating system credential store"),
    }
}

#[cfg(all(not(test), windows))]
fn windows_secret_path(config_path: &Path, slot: SecretSlot) -> PathBuf {
    let suffix = match slot {
        SecretSlot::Relay => WINDOWS_SECRET_SUFFIX,
        SecretSlot::ServiceRegistration => "service-registration",
    };
    config_path.with_extension(suffix)
}

#[cfg(all(not(test), windows))]
fn load_secret(config_path: &Path, slot: SecretSlot) -> Result<Option<String>> {
    let path = windows_secret_path(config_path, slot);
    let encrypted = match fs::read(&path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    decrypt_windows_secret(&encrypted).map(Some)
}

#[cfg(all(not(test), windows))]
fn store_secret(config_path: &Path, slot: SecretSlot, value: &str) -> Result<()> {
    let path = windows_secret_path(config_path, slot);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let protected = encrypt_windows_secret(value.as_bytes())?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "secret".to_string());
    let temporary = path.with_file_name(format!(".{file_name}.tmp"));
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
fn delete_secret(config_path: &Path, slot: SecretSlot) -> Result<()> {
    let path = windows_secret_path(config_path, slot);
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
    #[cfg(unix)]
    use super::{delete_development_secret, load_development_secret, store_development_secret};
    use super::{
        delete_relay_token, delete_service_registration_token, development_secret_path,
        load_relay_token, load_service_registration_token, store_relay_token,
        store_service_registration_token, SecretSlot,
    };
    use std::path::PathBuf;

    include!("secret_store_tests.rs");
}
