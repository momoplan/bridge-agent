use serde::Deserialize;
use url::Url;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialEnvironment {
    environment_key: String,
    api_base_url: String,
    legacy_api_origins: Vec<String>,
    legacy_api_paths: Vec<String>,
}

fn official() -> OfficialEnvironment {
    serde_json::from_str(include_str!("../../config/official-environment.json"))
        .expect("versioned official environment catalog")
}

pub fn official_api_base() -> String {
    official().api_base_url
}

/// Convert persisted legacy API prefixes to the environment API root. No HTTP
/// fallback is attempted; requests use only the current owner-service contract.
pub fn api_base(value: &str) -> String {
    let value = value.trim().trim_end_matches('/');
    let Ok(url) = Url::parse(value) else {
        return value.to_string();
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return value.to_string();
    }
    let catalog = official();
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    if (origin == catalog.api_base_url || catalog.legacy_api_origins.contains(&origin))
        && (path.is_empty() || catalog.legacy_api_paths.iter().any(|entry| entry == path))
    {
        return catalog.api_base_url;
    }
    // The retired service prefix is a protocol migration, not a SaaS host alias.
    // Preserve private origins, ports and any enclosing deployment path.
    value.strip_suffix("/lowcode3").unwrap_or(value).to_string()
}

pub fn official_key() -> String {
    official().environment_key
}
pub fn is_official(base_url: &str) -> bool {
    api_base(base_url) == official().api_base_url
}
pub fn bound_key(key: Option<&str>, base_url: &str) -> Option<String> {
    key.filter(|key| !key.is_empty() && key.trim() == *key)
        .map(str::to_owned)
        .or_else(|| (key.is_none() && is_official(base_url)).then(official_key))
}
