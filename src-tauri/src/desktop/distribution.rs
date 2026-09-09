use super::*;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClientDistributionConfig {
    schema_version: String,
    market_api_base_url: String,
}

pub(super) fn market_distribution_base_url() -> Result<reqwest::Url, String> {
    let config: ClientDistributionConfig =
        serde_json::from_str(include_str!("../../distribution-config.json"))
            .map_err(|err| format!("客户端市场分发配置无效: {err}"))?;
    if config.schema_version != "1.0.0" {
        return Err("客户端市场分发配置版本不受支持".to_string());
    }
    validate_market_distribution_url(&config.market_api_base_url)
}

fn validate_market_distribution_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|err| format!("市场分发地址无效: {err}"))?;
    if value.trim() != value
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("市场分发地址必须为不含凭据、查询参数或片段的 HTTPS 地址".to_string());
    }
    Ok(url)
}

pub(super) fn public_market_version_url(
    mut base: reqwest::Url,
    identity: &RegisteredAppVersionIdentity,
) -> Result<reqwest::Url, String> {
    base.path_segments_mut()
        .map_err(|_| "市场分发地址不能作为路径基址".to_string())?
        .pop_if_empty()
        .extend([
            "apps",
            &identity.app_id,
            "versions",
            &identity.version.to_string(),
        ]);
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_version_preserves_configured_path_and_exact_identity() {
        let identity =
            RegisteredAppVersionIdentity::parse("example/app".into(), "1.2.3+build.1".into())
                .unwrap();
        let url = public_market_version_url(
            validate_market_distribution_url("https://catalog.example.test/distribution/").unwrap(),
            &identity,
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://catalog.example.test/distribution/apps/example%2Fapp/versions/1.2.3+build.1"
        );
    }

    #[test]
    fn distribution_rejects_ambiguous_or_credential_bearing_configuration() {
        for url in [
            "http://example.test",
            "https://u:p@example.test",
            "https://example.test?token=value",
            "https://example.test#fragment",
            " https://example.test",
            "/relative",
        ] {
            assert!(validate_market_distribution_url(url).is_err(), "{url}");
        }
        assert!(market_distribution_base_url().is_ok());
    }
}
