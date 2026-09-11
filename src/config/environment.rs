use serde::Deserialize;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialEnvironment {
    environment_key: String,
    api_base_url: String,
}
fn official() -> OfficialEnvironment {
    serde_json::from_str(include_str!("../../config/official-environment.json"))
        .expect("versioned official environment catalog")
}
pub fn api_base(value: &str) -> String {
    let value = value.trim_end_matches('/');
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
