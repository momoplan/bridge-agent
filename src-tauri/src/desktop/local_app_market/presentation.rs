struct MarketArtifactPresentation {
    source: String,
    checksum: Option<String>,
    archive_path: Option<String>,
}

fn market_artifact_presentation(version: &RawMarketConnectorVersion) -> MarketArtifactPresentation {
    let artifact = select_market_tool_artifact(&version.manifest);
    let source = artifact
        .as_ref()
        .and_then(|artifact| artifact.get("source"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| market_connector_source(version));
    let checksum = artifact
        .as_ref()
        .and_then(|artifact| artifact.get("checksum"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or(version.checksum.clone());
    let archive_path = artifact
        .as_ref()
        .and_then(|artifact| artifact.get("archivePath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    MarketArtifactPresentation {
        source,
        checksum,
        archive_path,
    }
}

pub(super) fn market_release_notes(manifest: &Value) -> Vec<String> {
    ["releaseNotes", "changes", "changelog"]
        .iter()
        .find_map(|field| {
            manifest
                .get(field)
                .and_then(normalized_market_release_notes)
        })
        .unwrap_or_default()
}

pub(super) fn normalized_market_release_notes(value: &Value) -> Option<Vec<String>> {
    let values = match value {
        Value::String(value) => value.lines().map(str::to_string).collect::<Vec<_>>(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => return None,
    };
    let values = values
        .into_iter()
        .map(|value| {
            value
                .trim()
                .trim_start_matches(['-', '*', '•'])
                .trim()
                .to_string()
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

pub(super) fn market_manifest_database(manifest: &Value) -> Option<ConnectorDatabaseContract> {
    manifest
        .get("database")
        .cloned()
        .and_then(|database| serde_json::from_value(database).ok())
}

pub(super) fn market_contract_declaration(
    manifest: &Value,
    field: &str,
    contract_present: bool,
    application_type: &str,
) -> String {
    if contract_present {
        return "declared".to_string();
    }
    manifest
        .get("upgradeReview")
        .and_then(|review| review.get(field))
        .and_then(Value::as_str)
        .filter(|status| matches!(*status, "declared" | "not_applicable"))
        .map(str::to_string)
        .unwrap_or_else(|| {
            if application_type == "managed_tool" {
                "not_applicable".to_string()
            } else {
                "undeclared".to_string()
            }
        })
}

pub(super) fn market_manifest_method_contracts(manifest: &Value) -> Vec<ConnectorMethodContract> {
    market_manifest_contract_entries(manifest, "methods")
        .into_iter()
        .filter_map(|entry| {
            let name = market_manifest_text(entry, "name")?;
            Some(ConnectorMethodContract {
                name,
                description: market_manifest_text(entry, "description").unwrap_or_default(),
                input_schema: entry
                    .get("inputSchema")
                    .or_else(|| entry.get("input_schema"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
                response_mode: market_manifest_text(entry, "responseMode")
                    .or_else(|| market_manifest_text(entry, "response_mode"))
                    .unwrap_or_else(|| "cmodel".to_string()),
                path: market_manifest_text(entry, "path").unwrap_or_default(),
                http_method: market_manifest_text(entry, "httpMethod")
                    .or_else(|| market_manifest_text(entry, "http_method"))
                    .map(|method| method.to_uppercase())
                    .unwrap_or_else(|| "POST".to_string()),
            })
        })
        .collect()
}

pub(super) fn market_manifest_event_contracts(manifest: &Value) -> Vec<ConnectorEventContract> {
    market_manifest_contract_entries(manifest, "events")
        .into_iter()
        .filter_map(|entry| {
            let name = market_manifest_text(entry, "name")?;
            Some(ConnectorEventContract {
                name,
                description: market_manifest_text(entry, "description").unwrap_or_default(),
                payload_schema: entry
                    .get("payloadSchema")
                    .or_else(|| entry.get("payload_schema"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
            })
        })
        .collect()
}

pub(super) fn market_manifest_contract_entries<'a>(
    manifest: &'a Value,
    field: &str,
) -> Vec<&'a Value> {
    let mut entries = manifest
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(services) = manifest.get("services").and_then(Value::as_array) {
        for service in services {
            entries.extend(
                service
                    .get(field)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            );
        }
    }
    entries
}

pub(super) fn market_manifest_text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn market_manifest_permissions(manifest: &Value) -> Vec<ConnectorPermission> {
    manifest
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|permission| serde_json::from_value(permission.clone()).ok())
        .collect()
}

#[derive(Debug, Clone)]
pub(super) struct MarketHostCompatibility {
    pub(super) compatible: bool,
    pub(super) message: Option<String>,
    pub(super) minimum_host_version: Option<String>,
    pub(super) required_capabilities: Vec<String>,
    pub(super) missing_capabilities: Vec<String>,
}

pub(super) fn market_host_compatibility(
    manifest: &Value,
    server: Option<&RawMarketHostCompatibility>,
) -> MarketHostCompatibility {
    let requirements = manifest.get("hostRequirements").and_then(Value::as_object);
    let minimum_host_version = requirements
        .and_then(|value| value.get("minimumVersion"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| server.and_then(|value| value.minimum_host_version.clone()));
    let required_capabilities = requirements
        .and_then(|value| value.get("capabilities"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .or_else(|| server.map(|value| value.required_capabilities.clone()))
        .unwrap_or_default();
    let current_version = Version::parse(env!("CARGO_PKG_VERSION")).ok();
    let required_version = minimum_host_version
        .as_deref()
        .and_then(|value| Version::parse(value).ok());
    let version_compatible = match (required_version.as_ref(), current_version.as_ref()) {
        (Some(required), Some(current)) => current >= required,
        (Some(_), None) => false,
        (None, _) => true,
    };
    let supported_capabilities = LOCAL_APP_HOST_CAPABILITIES
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let missing_host_capabilities = required_capabilities
        .iter()
        .filter(|capability| !supported_capabilities.contains(capability.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let locally_compatible = version_compatible && missing_host_capabilities.is_empty();
    let compatible = locally_compatible && server.is_none_or(|value| value.compatible);
    let message = if compatible {
        None
    } else {
        server
            .and_then(|value| value.message.clone())
            .or_else(|| {
                (!version_compatible).then(|| {
                    format!(
                        "需要百积木客户端 {} 或更高版本，当前版本为 {}，请先升级客户端",
                        minimum_host_version.as_deref().unwrap_or("最新"),
                        env!("CARGO_PKG_VERSION")
                    )
                })
            })
            .or_else(|| {
                (!missing_host_capabilities.is_empty()).then(|| {
                    format!(
                        "当前百积木客户端缺少所需能力：{}，请先升级客户端",
                        missing_host_capabilities.join("、")
                    )
                })
            })
            .or_else(|| Some("当前百积木客户端不支持该应用版本，请先升级客户端".to_string()))
    };
    MarketHostCompatibility {
        compatible,
        message,
        minimum_host_version,
        required_capabilities,
        missing_capabilities: server
            .map(|value| value.missing_capabilities.clone())
            .filter(|values| !values.is_empty())
            .unwrap_or(missing_host_capabilities),
    }
}

pub(super) fn validate_market_host_compatibility(
    market_app: &MarketConnectorApp,
) -> Result<(), String> {
    if market_app.compatible {
        return Ok(());
    }
    Err(market_app
        .compatibility_message
        .clone()
        .unwrap_or_else(|| "当前百积木客户端不支持该应用版本，请先升级客户端".to_string()))
}

pub(super) fn validate_market_app_identity(
    market_app: &MarketConnectorApp,
    app_id: &str,
) -> Result<(), String> {
    if market_app.application_type != "connector" {
        return Err("该市场条目不是 Connector 应用".to_string());
    }
    if market_app.app_id.trim() != app_id.trim() {
        return Err(format!(
            "市场应用 ID 与安装包不匹配：市场 `{}`，安装包 `{}`",
            market_app.app_id, app_id
        ));
    }
    if !market_app.source.trim().starts_with("https://") {
        return Err("市场 Connector 安装源必须使用 HTTPS".to_string());
    }
    Ok(())
}

pub(super) fn required_market_checksum(market_app: &MarketConnectorApp) -> Result<String, String> {
    let checksum = market_app
        .checksum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "市场本地应用发布包必须提供 SHA-256 checksum".to_string())?;
    let digest = checksum.strip_prefix("sha256:").unwrap_or(checksum);
    if digest.len() != 64
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("市场本地应用 SHA-256 checksum 格式无效".to_string());
    }
    Ok(format!("sha256:{}", digest.to_ascii_lowercase()))
}

pub(super) fn select_market_tool_artifact(manifest: &Value) -> Option<Value> {
    let platform = normalized_platform();
    let arch = std::env::consts::ARCH;
    manifest
        .get("artifacts")?
        .as_array()?
        .iter()
        .find(|artifact| {
            let artifact_platform = artifact
                .get("platform")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let artifact_arch = artifact
                .get("arch")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("universal");
            artifact_platform.eq_ignore_ascii_case(platform)
                && (artifact_arch.eq_ignore_ascii_case(arch)
                    || artifact_arch.eq_ignore_ascii_case("universal")
                    || (arch == "x86_64" && artifact_arch.eq_ignore_ascii_case("x64"))
                    || (arch == "aarch64" && artifact_arch.eq_ignore_ascii_case("arm64")))
        })
        .cloned()
}

pub(super) fn market_connector_source(version: &RawMarketConnectorVersion) -> String {
    let source_type = version
        .source_type
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if source_type.eq_ignore_ascii_case("git") || is_git_connector_source(&version.source) {
        with_revision(&version.source, version.revision.as_deref())
    } else {
        version.source.trim().to_string()
    }
}

pub(super) fn with_revision(source: &str, revision: Option<&str>) -> String {
    let source = source.trim();
    match revision.map(str::trim).filter(|value| !value.is_empty()) {
        Some(revision) if !source.contains('#') => format!("{source}#{revision}"),
        _ => source.to_string(),
    }
}

pub(super) fn split_source_revision(source: &str) -> (String, Option<String>) {
    let source = source.trim();
    match source.rsplit_once('#') {
        Some((base, revision)) if !base.is_empty() && !revision.is_empty() => {
            (base.to_string(), Some(revision.to_string()))
        }
        _ => (source.to_string(), None),
    }
}

pub(super) fn normalized_platform() -> &'static str {
    std::env::consts::OS
}

pub(super) fn is_git_connector_source(source: &str) -> bool {
    let value = source.trim();
    value.starts_with("git@")
        || value.ends_with(".git")
        || value.starts_with("ssh://")
        || value.starts_with("git://")
        || parse_https_git_repo(value, "github.com").is_some()
        || parse_https_git_repo(value, "gitee.com").is_some()
}

pub(super) fn is_http_connector_source(source: &str) -> bool {
    let value = source.trim();
    value.starts_with("https://") || value.starts_with("http://")
}

pub(super) fn connector_version_is_newer(latest: &str, current: &str) -> bool {
    let latest = latest.trim().trim_start_matches('v');
    let current = current.trim().trim_start_matches('v');
    match (Version::parse(latest), Version::parse(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => latest != current,
    }
}
