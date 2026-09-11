use super::*;

pub(super) fn validate_shared_environment_binding(
    document: &Value,
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) -> anyhow::Result<()> {
    use bridge_agent::config::environment::{api_base, is_official, official_key};
    anyhow::ensure!(
        is_official(&config.platform.base_url) == (authorized.environment_key == official_key()),
        "授权地址与官方环境身份不匹配"
    );
    if let Some(environments) = document.get("environments").and_then(Value::as_object) {
        for environment in environments.values() {
            if environment.get("environmentKey").and_then(Value::as_str)
                == Some(authorized.environment_key.as_str())
            {
                anyhow::ensure!(
                    environment
                        .get("baseUrl")
                        .and_then(Value::as_str)
                        .is_some_and(|url| api_base(url) == api_base(&config.platform.base_url)),
                    "该环境身份已绑定另一个 API 地址，不能覆盖现有凭据的连接地址"
                );
            }
        }
    }
    Ok(())
}

pub(super) fn configure_shared_cli_environment(
    document: &mut Value,
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) {
    if !document.get("environments").is_some_and(Value::is_object) {
        document["environments"] = serde_json::json!({});
    }
    // Bridge owns its device binding, never the CLI's current selection.
    document["environments"][&authorized.environment_key] = serde_json::json!({
        "environmentKey": authorized.environment_key,
        "baseUrl": bridge_agent::config::environment::api_base(&config.platform.base_url),
    });
}

pub(super) fn shared_credential_matches_environment(
    item: &Value,
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) -> bool {
    use bridge_agent::config::environment::{api_base, is_official, official_key};
    item.get("environmentKey")
        .and_then(Value::as_str)
        .map_or_else(
            || {
                item.get("baseUrl").and_then(Value::as_str).map_or_else(
                    || {
                        authorized.environment_key == official_key()
                            && is_official(&config.platform.base_url)
                    },
                    |url| api_base(url) == api_base(&config.platform.base_url),
                )
            },
            |key| key == authorized.environment_key,
        )
}
