//! Read only the credential issued for this device, never the CLI's active login.
use super::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    schema_version: u32,
    credentials: Vec<Credential>,
    environments: BTreeMap<String, Environment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Environment {
    environment_key: Option<String>,
    base_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    environment_key: Option<String>,
    client_id: Option<String>,
    #[serde(default)]
    workspace_ids: Vec<u64>,
    user_id: Option<u64>,
    source: Option<String>,
    subject_type: Option<String>,
    token_type: Option<String>,
    token: String,
}

pub(super) struct DeviceCredential {
    pub token: String,
    pub user_id: u64,
}

pub(super) fn path() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("BAIJIMU_CONFIG_HOME") {
        return Ok(PathBuf::from(root).join("baijimu/auth.json"));
    }
    let home =
        directories::BaseDirs::new().context("cannot locate user configuration directory")?;
    Ok(home.home_dir().join(".config/baijimu/auth.json"))
}

pub(super) fn read(path: &Path, binding: &PersonalFileClient) -> Result<DeviceCredential> {
    let bytes = std::fs::read(path)
        .context("device user credential unavailable; authorize this device again")?;
    let document: Document = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid device user credential document"))?;
    anyhow::ensure!(
        document.schema_version == 2,
        "unsupported device credential schema"
    );
    let workspace = binding
        .workspace_id
        .context("device has no workspace authorization")?;
    let environment = binding
        .environment_key
        .as_deref()
        .context("device has no environment authorization")?;
    let addresses: Vec<_> = document
        .environments
        .values()
        .filter(|entry| entry.environment_key.as_deref() == Some(environment))
        .collect();
    anyhow::ensure!(
        !addresses.is_empty()
            && addresses.iter().all(
                |entry| crate::config::environment::api_base(&entry.base_url) == binding.base_url
            ),
        "device credential API environment binding mismatch"
    );
    let mut matches = document.credentials.into_iter().filter(|entry| {
        entry.environment_key.as_deref() == Some(environment)
            && entry.client_id.as_deref() == Some(binding.device_id.as_str())
            && entry.workspace_ids == [workspace]
            && entry.source.as_deref() == Some("bridge-agent")
    });
    let entry = matches
        .next()
        .context("device-bound user credential unavailable; authorize this device again")?;
    anyhow::ensure!(
        matches.next().is_none(),
        "ambiguous device-bound user credentials"
    );
    anyhow::ensure!(
        entry.subject_type.as_deref() == Some("user")
            && entry.token_type.as_deref() == Some("pat")
            && entry.token.starts_with("lc_pat_")
            && entry.token.trim() == entry.token,
        "invalid device-bound personal access credential"
    );
    let user_id = entry
        .user_id
        .filter(|id| *id > 0)
        .context("device credential has no authorizing user")?;
    Ok(DeviceCredential {
        token: entry.token,
        user_id,
    })
}
