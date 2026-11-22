use super::*;
use bridge_agent::market_distribution::{
    select_artifact, select_target_artifact, EnvironmentReader,
};
use local_app_contract::{bundle::LocatedAppVersion, SourceVersion};

pub(super) enum Selection<'a> {
    Legacy(&'a SourceVersion),
    Bundle(&'a LocatedAppVersion),
}

pub(super) async fn resolve(
    config_path: &Path,
    source: Selection<'_>,
    migrate: bool,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<(ResolvedConnectorSource, ConnectorInstallProvenance), String> {
    let config = load_agent_config(config_path).map_err(|error| error.to_string())?;
    let environment = config
        .platform
        .environment_key
        .as_deref()
        .ok_or("设备缺少已授权的来源环境身份")?;
    let source_environment = match &source {
        Selection::Legacy(source) => source.application.environment_key.as_str(),
        Selection::Bundle(source) => source.resource_locator.resource_locator().environment_key(),
    };
    if environment != source_environment {
        return Err("私有安装必须来自设备已授权的环境，不能切换或推断来源".into());
    }
    let bytes = fs::read(shared_cli_auth_path()).map_err(|_| "无法读取环境授权")?;
    let document = serde_json::from_slice(&bytes).map_err(|_| "环境授权格式无效")?;
    let credential = market_consumer::credential_for(&config, document)?;
    let endpoint = environment_api_base(&config.platform.base_url)?;
    let reader = EnvironmentReader::new(
        environment
            .to_owned()
            .try_into()
            .map_err(|error: local_app_contract::ContractError| error.to_string())?,
        endpoint.as_str(),
        config
            .platform
            .workspace_id
            .ok_or("设备缺少已授权的工作区身份")?,
        Duration::from_secs(60),
    )
    .map_err(|error| error.to_string())?;
    let (artifact, mut response, provenance) =
        open_frozen_connector(&reader, &credential, source, migrate).await?;
    let kind = connector_archive_kind(artifact.file_name.as_str()).ok_or("安装包格式不受支持")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        bridge_agent::market_distribution::artifact_read_error(error, &response).to_string()
    })? {
        if (bytes.len() as u64)
            .checked_add(chunk.len() as u64)
            .is_none_or(|size| size > artifact.size_bytes)
        {
            return Err("环境应用制品超过冻结版本声明的大小".into());
        }
        bytes.extend_from_slice(&chunk);
        if let Some(progress) = progress {
            progress.download(bytes.len() as u64, Some(artifact.size_bytes));
        }
    }
    if bytes.len() as u64 != artifact.size_bytes {
        return Err("环境应用制品下载不完整".into());
    }
    let temp_dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let extract_dir = temp_dir.path().join("package");
    fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
    extract_connector_archive(&bytes, kind, &extract_dir)?;
    let path = find_extracted_connector_root(&extract_dir)?;
    let manifest = load_connector_manifest(&path).map_err(|error| error.to_string())?;
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

async fn open_frozen_connector(
    reader: &EnvironmentReader,
    credential: &str,
    source: Selection<'_>,
    migrate: bool,
) -> Result<
    (
        local_app_contract::Artifact,
        reqwest::Response,
        ConnectorInstallProvenance,
    ),
    String,
> {
    Ok(match source {
        Selection::Legacy(source) => {
            let frozen = reader
                .version(credential, source)
                .await
                .map_err(|error| error.to_string())?;
            let artifact = select_artifact(&frozen, normalized_platform(), std::env::consts::ARCH)
                .map_err(|error| error.to_string())?
                .ok_or("该来源版本不支持当前平台")?;
            let response = reader
                .artifact(credential, &frozen, artifact.artifact_id)
                .await
                .map_err(|error| error.to_string())?;
            let provenance = ConnectorInstallProvenance::frozen_environment(&frozen, migrate)
                .map_err(|error| error.to_string())?;
            (artifact.clone(), response, provenance)
        }
        Selection::Bundle(source) => {
            let frozen = reader
                .located_version(credential, source)
                .await
                .map_err(|error| error.to_string())?;
            let artifact = select_target_artifact(
                &frozen.content,
                normalized_platform(),
                std::env::consts::ARCH,
            )
            .ok_or("该来源版本不支持当前平台")?;
            let response = reader
                .located_artifact(credential, &frozen, artifact.artifact_id)
                .await
                .map_err(|error| error.to_string())?;
            let mut provenance = ConnectorInstallProvenance::frozen_bundle(&frozen)
                .map_err(|error| error.to_string())?;
            if migrate {
                let key = source.resource_locator.resource_locator().resource_key();
                let installed = show_connector(key).map_err(|error| error.to_string())?;
                let legacy = match installed.install_source {
                    Some(
                        local_app_contract::InstallSource::Market { source, .. }
                        | local_app_contract::InstallSource::Environment { source },
                    ) => source,
                    None => return Err("旧安装缺少已验证来源，无法迁移".into()),
                };
                let adoption = reader
                    .adoption(credential, &legacy)
                    .await
                    .map_err(|error| error.to_string())?;
                if adoption != *source {
                    return Err("来源登记的迁入 Locator 或精确版本与所选目标不一致".into());
                }
                provenance = provenance
                    .for_adoption(legacy)
                    .map_err(|error| error.to_string())?;
            }
            (artifact.clone(), response, provenance)
        }
    })
}

fn environment_api_base(platform_base: &str) -> Result<reqwest::Url, String> {
    let mut endpoint = market_consumer::consumer_service_api_base(platform_base)?;
    endpoint
        .path_segments_mut()
        .map_err(|_| "环境地址无效")?
        .pop_if_empty()
        .push("local-apps");
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_frozen_versions_keep_their_own_route_when_market_uses_v4() {
        for (platform, expected) in [
            (
                "https://consumer.example.test/lowcode3",
                "https://consumer.example.test/partner/v1/local-app-service/api/local-apps",
            ),
            (
                "https://consumer.example.test/env/lowcode3/",
                "https://consumer.example.test/env/partner/v1/local-app-service/api/local-apps",
            ),
            (
                "https://consumer.example.test/env",
                "https://consumer.example.test/env/partner/v1/local-app-service/api/local-apps",
            ),
        ] {
            let endpoint = environment_api_base(platform).unwrap();
            assert_eq!(endpoint.as_str(), expected);
            let source: SourceVersion = serde_json::from_value(serde_json::json!({
                "application": {"environmentKey": "consumer", "appId": "test-app"},
                "version": "1.2.3"
            }))
            .unwrap();
            let reader = EnvironmentReader::new(
                source.application.environment_key.clone(),
                endpoint.as_str(),
                7,
                Duration::from_secs(60),
            )
            .unwrap();
            assert_eq!(
                reader.version_url(&source).unwrap().as_str(),
                format!("{expected}/test-app/versions/1.2.3")
            );
        }
    }
}
