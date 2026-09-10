use super::*;
use bridge_agent::market_distribution::MarketReader;
use local_app_contract::MarketListing;
use std::collections::BTreeMap;

pub(super) struct MarketConsumer {
    pub(super) reader: MarketReader,
    pub(super) credential: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedAuthentication {
    current_environment: String,
    current_workspace_id: u64,
    environments: BTreeMap<String, ConsumerEnvironment>,
    credentials: Vec<ConsumerCredential>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsumerEnvironment {
    base_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsumerCredential {
    base_url: Option<String>,
    client_id: Option<String>,
    token: String,
    token_type: String,
    workspace_ids: Vec<u64>,
    source: Option<String>,
}

fn credential_for(config: &AgentConfig, document: SharedAuthentication) -> Result<String, String> {
    let workspace = config.platform.workspace_id.ok_or("请先完成工作区授权")?;
    let environment = document
        .environments
        .get(&document.current_environment)
        .ok_or("当前授权缺少环境绑定")?;
    if document.current_workspace_id != workspace
        || environment.base_url.trim_end_matches('/')
            != config.platform.base_url.trim_end_matches('/')
    {
        return Err("当前工作区授权与客户端环境不一致，请重新授权".into());
    }
    let mut matches = document.credentials.into_iter().filter(|credential| {
        credential.base_url.as_deref().is_some_and(|url| {
            url.trim_end_matches('/') == config.platform.base_url.trim_end_matches('/')
        }) && credential.client_id.as_deref() == Some(config.relay.agent_id.as_str())
            && credential.source.as_deref() == Some("bridge-agent")
            && credential.token_type == "pat"
            && credential.workspace_ids.contains(&workspace)
    });
    let credential = matches
        .next()
        .ok_or("缺少当前设备的工作区读取凭证，请重新授权")?;
    if matches.next().is_some()
        || credential.token.is_empty()
        || credential.token.trim() != credential.token
    {
        return Err("当前设备的工作区读取凭证不唯一或无效".into());
    }
    Ok(credential.token)
}

pub(super) async fn market_consumer(config_path: &Path) -> Result<MarketConsumer, String> {
    let config = load_agent_config(config_path).map_err(|error| error.to_string())?;
    let bytes =
        fs::read(shared_cli_auth_path()).map_err(|_| "无法读取当前工作区授权，请重新授权")?;
    let document = serde_json::from_slice(&bytes).map_err(|_| "当前工作区授权文件无效")?;
    let credential = credential_for(&config, document)?;
    let base = consumer_api_base(&config.platform.base_url)?;
    let reader = MarketReader::connect(
        base.as_str(),
        config.platform.workspace_id.unwrap(),
        &credential,
        Duration::from_secs(60),
    )
    .await
    .map_err(|error| format!("读取环境本地应用市场失败：{error}"))?;
    Ok(MarketConsumer { reader, credential })
}

// platform.base_url may name the lowcode API; Partner routes share its parent base.
fn consumer_api_base(platform_base: &str) -> Result<reqwest::Url, String> {
    let base = platform_base.trim_end_matches('/');
    let base = base.strip_suffix("/lowcode3").unwrap_or(base);
    let mut url = reqwest::Url::parse(base).map_err(|_| "当前环境地址无效")?;
    url.path_segments_mut()
        .map_err(|_| "当前环境地址不能作为路径基址")?
        .pop_if_empty()
        .extend([
            "partner",
            "v1",
            "local-app-service",
            "api",
            "local-app-market",
            "",
        ]);
    Ok(url)
}

impl MarketConsumer {
    pub(super) async fn listings(&self) -> Result<Vec<MarketListing>, String> {
        let mut cursor = None;
        let mut items = Vec::new();
        loop {
            let page = self
                .reader
                .page(&self.credential, cursor)
                .await
                .map_err(|error| error.to_string())?;
            items.extend(page.items);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(items),
            }
        }
    }
}

pub(super) async fn resolve_market_install(
    config_path: &Path,
    identity: &RegisteredAppVersionIdentity,
    selection: &local_app_contract::InstallSource,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<(ResolvedConnectorSource, ConnectorInstallProvenance), String> {
    let consumer = market_consumer(config_path).await?;
    let listing = consumer
        .reader
        .version(&consumer.credential, selection)
        .await
        .map_err(|error| error.to_string())?;
    if listing.frozen_version.source.application.app_id.as_str() != identity.app_id
        || listing.frozen_version.source.version.to_string() != identity.version.to_string()
    {
        return Err("选择的市场来源与请求应用或版本不一致".into());
    }
    validate_market_host_compatibility(&market_listing_presentation(listing.clone())?)?;
    let artifact = bridge_agent::market_distribution::select_artifact(
        &listing.frozen_version,
        normalized_platform(),
        std::env::consts::ARCH,
    )
    .map_err(|error| error.to_string())?
    .ok_or("该版本未提供当前平台的安装包")?;
    let kind =
        connector_archive_kind(artifact.file_name.as_str()).ok_or("当前应用安装包格式不受支持")?;
    let mut response = consumer
        .reader
        .artifact(&consumer.credential, selection, artifact.artifact_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取环境应用制品失败")? {
        if (bytes.len() as u64)
            .checked_add(chunk.len() as u64)
            .is_none_or(|size| size > artifact.size_bytes)
        {
            return Err("应用制品超过冻结版本声明的大小".into());
        }
        bytes.extend_from_slice(&chunk);
        if let Some(progress) = progress {
            progress.download(bytes.len() as u64, Some(artifact.size_bytes));
        }
    }
    if bytes.len() as u64 != artifact.size_bytes {
        return Err("应用制品下载不完整".into());
    }
    let temp_dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let extract_dir = temp_dir.path().join("package");
    fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
    extract_connector_archive(&bytes, kind, &extract_dir)?;
    let path = find_extracted_connector_root(&extract_dir)?;
    let package_manifest: Value = serde_json::from_slice(
        &fs::read(path.join("connector.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let frozen_manifest: Value =
        serde_json::from_str(listing.frozen_version.content.manifest.as_json())
            .map_err(|error| error.to_string())?;
    if package_manifest != frozen_manifest {
        return Err("安装包清单与已审核的冻结版本不一致".into());
    }
    let manifest = load_connector_manifest(&path).map_err(|error| error.to_string())?;
    let provenance = ConnectorInstallProvenance::frozen_market(selection.clone(), &listing)
        .map_err(|error| error.to_string())?;
    provenance
        .validate_manifest(&manifest)
        .map_err(|error| error.to_string())?;
    Ok((
        ResolvedConnectorSource::Archive {
            path,
            _temp_dir: temp_dir,
        },
        provenance,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AgentConfig {
        let mut config = AgentConfig::example();
        config.platform.base_url = "https://consumer.example.test".into();
        config.platform.workspace_id = Some(7);
        config.relay.agent_id = "device-a".into();
        config
    }
    fn document() -> Value {
        serde_json::json!({
            "currentEnvironment":"consumer", "currentWorkspaceId":7,
            "environments":{"consumer":{"baseUrl":"https://consumer.example.test"}},
            "credentials":[{"baseUrl":"https://consumer.example.test", "clientId":"device-a",
                "source":"bridge-agent", "token":"test-pat", "tokenType":"pat", "workspaceIds":[7]}]
        })
    }
    fn resolve(document: Value) -> Result<String, String> {
        credential_for(&config(), serde_json::from_value(document).unwrap())
    }
    #[test]
    fn consumer_endpoint_uses_partner_base_and_preserves_environment_prefix() {
        for (base, expected) in [
            ("https://consumer.example.test/lowcode3", "https://consumer.example.test/partner/v1/local-app-service/api/local-app-market/"),
            ("https://consumer.example.test/env/lowcode3/", "https://consumer.example.test/env/partner/v1/local-app-service/api/local-app-market/"),
            ("https://consumer.example.test/env", "https://consumer.example.test/env/partner/v1/local-app-service/api/local-app-market/"),
        ] {
            assert_eq!(consumer_api_base(base).unwrap().as_str(), expected);
        }
    }

    #[test]
    fn credential_is_bound_to_consumer_environment_workspace_and_device() {
        assert_eq!(resolve(document()).unwrap(), "test-pat");
        for (field, value) in [
            ("baseUrl", Value::from("https://author.example.test")),
            ("clientId", Value::from("device-b")),
            ("workspaceIds", serde_json::json!([8])),
            ("tokenType", Value::from("jwt")),
            ("baseUrl", Value::Null),
        ] {
            let mut document = document();
            document["credentials"][0][field] = value;
            assert!(resolve(document).is_err(), "{field}");
        }
    }
    #[test]
    fn credential_selection_rejects_ambiguous_or_switched_workspace() {
        let mut value = document();
        let duplicate = value["credentials"][0].clone();
        value["credentials"].as_array_mut().unwrap().push(duplicate);
        assert!(resolve(value).is_err());
        let mut value = document();
        value["currentWorkspaceId"] = 8.into();
        assert!(resolve(value).is_err());
    }
}
