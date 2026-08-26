use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MarketConnectorApp {
    pub(super) app_id: String,
    pub(super) application_type: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) source: String,
    pub(super) repo: String,
    pub(super) revision: String,
    pub(super) checksum: Option<String>,
    pub(super) archive_path: Option<String>,
    pub(super) risk: String,
    pub(super) risk_level: String,
    pub(super) capability: String,
    pub(super) version: String,
    pub(super) published_at: Option<String>,
    pub(super) icon_data_url: Option<String>,
    pub(super) release_notes: Vec<String>,
    pub(super) configuration_declaration: String,
    pub(super) interface_declaration: String,
    pub(super) database_declaration: String,
    pub(super) config_schema: Option<Value>,
    pub(super) database: Option<ConnectorDatabaseContract>,
    pub(super) methods: Vec<ConnectorMethodContract>,
    pub(super) events: Vec<ConnectorEventContract>,
    pub(super) method_names: Vec<String>,
    pub(super) event_names: Vec<String>,
    pub(super) permissions: Vec<ConnectorPermission>,
    pub(super) compatible: bool,
    pub(super) compatibility_message: Option<String>,
    pub(super) minimum_host_version: Option<String>,
    pub(super) required_host_capabilities: Vec<String>,
    pub(super) missing_host_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawLocalAppMarketResponse<T> {
    pub(super) error_code: Option<String>,
    pub(super) value: Option<String>,
    pub(super) data: Option<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawMarketConnectorApp {
    pub(super) app_id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) risk: String,
    pub(super) risk_level: Option<String>,
    pub(super) capability: String,
    pub(super) latest_version: RawMarketConnectorVersion,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawMarketConnectorVersion {
    pub(super) version: String,
    pub(super) source: String,
    pub(super) source_type: Option<String>,
    pub(super) repo: Option<String>,
    pub(super) revision: Option<String>,
    pub(super) checksum: Option<String>,
    pub(super) published_at: Option<String>,
    #[serde(default)]
    pub(super) manifest: Value,
    #[serde(default)]
    pub(super) compatibility: Option<RawMarketHostCompatibility>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RawRegisteredLocalApp {
    pub(super) app_id: String,
    pub(super) registration_status: String,
    pub(super) review_status: String,
    pub(super) name: String,
    pub(super) publisher: String,
    pub(super) platforms: Vec<String>,
    pub(super) version: RawMarketConnectorVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RegisteredAppVersionIdentity {
    pub(super) app_id: String,
    pub(super) version: Version,
}

impl RegisteredAppVersionIdentity {
    pub(super) fn parse(app_id: String, version: String) -> Result<Self, String> {
        if app_id.is_empty() || app_id.trim() != app_id {
            return Err("appId 不能为空或包含首尾空白".to_string());
        }
        let parsed_version = Version::parse(&version)
            .map_err(|err| format!("本地应用版本必须是严格 SemVer 2.0.0：{err}"))?;
        if parsed_version.to_string() != version {
            return Err("本地应用版本必须使用规范 SemVer 2.0.0 表达".to_string());
        }
        Ok(Self {
            app_id,
            version: parsed_version,
        })
    }
}

#[derive(Debug)]
pub(super) struct RegisteredInstallSource {
    pub(super) identity: RegisteredAppVersionIdentity,
    pub(super) review_status: String,
    pub(super) name: String,
    pub(super) publisher: String,
    pub(super) source: String,
    pub(super) checksum: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RawMarketHostCompatibility {
    pub(super) compatible: bool,
    pub(super) message: Option<String>,
    pub(super) minimum_host_version: Option<String>,
    #[serde(default)]
    pub(super) required_capabilities: Vec<String>,
    #[serde(default)]
    pub(super) missing_capabilities: Vec<String>,
}

#[tauri::command]
pub(super) async fn list_market_connector_apps(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<MarketConnectorApp>, String> {
    fetch_market_connector_apps(&state.config_path).await
}

pub(super) async fn fetch_market_connector_apps(
    config_path: &Path,
) -> Result<Vec<MarketConnectorApp>, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let base_url = config.platform.base_url.trim_end_matches('/');
    let platform = normalized_platform();
    let arch = std::env::consts::ARCH;
    let mut url = reqwest::Url::parse(&format!("{base_url}/api/local-app-market/apps"))
        .map_err(|err| format!("localApp 市场地址无效: {err}"))?;
    url.query_pairs_mut()
        .append_pair("platform", platform)
        .append_pair("arch", arch)
        .append_pair("hostVersion", env!("CARGO_PKG_VERSION"))
        .append_pair("hostCapabilities", &LOCAL_APP_HOST_CAPABILITIES.join(","));
    let response = Client::new()
        .get(url)
        .send()
        .await
        .map_err(|err| format!("请求 localApp 市场失败: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("请求 localApp 市场失败: HTTP {status} {body}"));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|err| format!("解析 localApp 市场响应失败: {err}"))?;
    let raw_apps: Vec<RawMarketConnectorApp> = if payload.get("data").is_some() {
        let wrapped: RawLocalAppMarketResponse<Vec<RawMarketConnectorApp>> =
            serde_json::from_value(payload)
                .map_err(|err| format!("解析 lowcode localApp 市场响应失败: {err}"))?;
        if wrapped
            .error_code
            .as_deref()
            .is_some_and(|code| code != "0")
        {
            return Err(format!(
                "lowcode localApp 市场返回失败: {}",
                wrapped.value.unwrap_or_else(|| "未知错误".to_string())
            ));
        }
        wrapped.data.unwrap_or_default()
    } else {
        serde_json::from_value(payload)
            .map_err(|err| format!("解析 local-app-market 响应失败: {err}"))?
    };
    Ok(raw_apps.into_iter().map(MarketConnectorApp::from).collect())
}

pub(super) fn registered_install_url(
    base_url: &str,
    identity: &RegisteredAppVersionIdentity,
) -> Result<reqwest::Url, String> {
    let version = identity.version.to_string();
    let mut url = reqwest::Url::parse(base_url.trim_end_matches('/'))
        .map_err(|err| format!("本地应用注册中心地址无效: {err}"))?;
    url.path_segments_mut()
        .map_err(|_| "本地应用注册中心地址不能作为路径基址".to_string())?
        .pop_if_empty()
        .extend([
            "api",
            "local-app-registry",
            "apps",
            identity.app_id.as_str(),
            "versions",
            version.as_str(),
        ]);
    Ok(url)
}

pub(super) async fn fetch_registered_install_source(
    config_path: &Path,
    identity: &RegisteredAppVersionIdentity,
) -> Result<RegisteredInstallSource, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let platform = normalized_platform();
    let arch = std::env::consts::ARCH;
    let mut url = registered_install_url(&config.platform.base_url, identity)?;
    url.query_pairs_mut()
        .append_pair("platform", platform)
        .append_pair("arch", arch)
        .append_pair("hostVersion", env!("CARGO_PKG_VERSION"))
        .append_pair("hostCapabilities", &LOCAL_APP_HOST_CAPABILITIES.join(","));
    let response = Client::new()
        .get(url)
        .send()
        .await
        .map_err(|err| format!("查询本地应用注册版本失败: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "应用 {}@{} 未注册、已撤销或不支持当前平台: HTTP {status} {body}",
            identity.app_id, identity.version
        ));
    }
    let registered: RawRegisteredLocalApp = response
        .json()
        .await
        .map_err(|err| format!("解析本地应用注册版本失败: {err}"))?;
    let registered_identity = RegisteredAppVersionIdentity::parse(
        registered.app_id.clone(),
        registered.version.version.clone(),
    )?;
    if registered_identity != *identity || registered.registration_status != "ACTIVE" {
        return Err("注册中心返回的应用身份或状态与请求不一致".to_string());
    }
    let _ = &registered.platforms;
    let checksum = registered
        .version
        .checksum
        .as_deref()
        .ok_or_else(|| "注册版本缺少安装 checksum".to_string())?;
    let digest = checksum.strip_prefix("sha256:").unwrap_or(checksum);
    if digest.len() != 64
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("注册版本 checksum 格式无效".to_string());
    }
    Ok(RegisteredInstallSource {
        identity: registered_identity,
        review_status: registered.review_status,
        name: registered.name,
        publisher: registered.publisher,
        source: registered.version.source,
        checksum: digest.to_ascii_lowercase(),
    })
}

pub(super) fn ensure_registered_install_is_accepted(
    registered: &RegisteredInstallSource,
    accept_unreviewed: bool,
) -> Result<(), String> {
    if registered.review_status == "PUBLISHED" || accept_unreviewed {
        return Ok(());
    }
    Err(format!(
        "应用 {}（{}@{}，发布者 {}）尚未经过市场公开审核；确认开发者和权限后显式允许安装未审核版本",
        registered.name,
        registered.identity.app_id,
        registered.identity.version,
        registered.publisher
    ))
}

#[tauri::command]
pub(super) async fn show_connector_app(id: String) -> Result<ConnectorInstallRecord, String> {
    show_connector(id.trim()).map_err(|err| err.to_string())
}

pub(super) enum ResolvedConnectorSource {
    Local(PathBuf),
    Git {
        path: PathBuf,
        _temp_dir: tempfile::TempDir,
    },
    Archive {
        path: PathBuf,
        _temp_dir: tempfile::TempDir,
    },
}

impl ResolvedConnectorSource {
    pub(super) fn path(&self) -> &Path {
        match self {
            Self::Local(path) => path.as_path(),
            Self::Git { path, .. } => path.as_path(),
            Self::Archive { path, .. } => path.as_path(),
        }
    }
}

pub(super) async fn resolve_connector_source(
    source: &str,
    allow_git: bool,
    expected_checksum: Option<&str>,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<ResolvedConnectorSource, String> {
    let (source, git_revision) = split_source_revision(source);
    if let Some(archive_url) =
        connector_archive_download_url(&source, git_revision.as_deref(), allow_git)?
    {
        return resolve_connector_archive_source(&archive_url, expected_checksum, progress).await;
    }

    if is_git_connector_source(&source) {
        if !allow_git {
            return Err(
                "市场本地应用不能依赖本机 git，请将安装源发布为 .zip 或 .tar.gz 下载包。"
                    .to_string(),
            );
        }
        let temp_dir = tempfile::tempdir().map_err(|err| err.to_string())?;
        let checkout_path = temp_dir.path().join("connector");
        let mut command = Command::new("git");
        configure_desktop_command(&mut command);
        command.args(["clone", "--depth", "1"]);
        if let Some(revision) = git_revision.as_deref().filter(|value| !value.is_empty()) {
            command.args(["--branch", revision]);
        }
        let output = command
            .arg(&source)
            .arg(&checkout_path)
            .output()
            .map_err(|err| format!("执行 git clone 失败: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "下载本地应用失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        return Ok(ResolvedConnectorSource::Git {
            path: checkout_path,
            _temp_dir: temp_dir,
        });
    }

    let path = PathBuf::from(source);
    if !path.exists() {
        return Err(format!("本地路径不存在: {}", path.display()));
    }
    Ok(ResolvedConnectorSource::Local(path))
}

impl From<RawMarketConnectorApp> for MarketConnectorApp {
    fn from(value: RawMarketConnectorApp) -> Self {
        let host_compatibility = market_host_compatibility(
            &value.latest_version.manifest,
            value.latest_version.compatibility.as_ref(),
        );
        let application_type = value
            .latest_version
            .manifest
            .get("applicationType")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("connector")
            .to_string();
        let artifact = market_artifact_presentation(&value.latest_version);
        let source = artifact.source;
        let checksum = artifact.checksum;
        let archive_path = artifact.archive_path;
        let release_notes = market_release_notes(&value.latest_version.manifest);
        let icon_data_url = value
            .latest_version
            .manifest
            .get("icon")
            .cloned()
            .and_then(|icon| serde_json::from_value::<ConnectorIcon>(icon).ok())
            .and_then(|icon| connector_icon_data_url(&icon).ok());
        let config_schema = value.latest_version.manifest.get("configSchema").cloned();
        let database = market_manifest_database(&value.latest_version.manifest);
        let methods = market_manifest_method_contracts(&value.latest_version.manifest);
        let events = market_manifest_event_contracts(&value.latest_version.manifest);
        let configuration_declaration = market_contract_declaration(
            &value.latest_version.manifest,
            "configuration",
            config_schema.is_some(),
            &application_type,
        );
        let interface_declaration = market_contract_declaration(
            &value.latest_version.manifest,
            "interfaces",
            !methods.is_empty() || !events.is_empty(),
            &application_type,
        );
        let database_declaration = market_contract_declaration(
            &value.latest_version.manifest,
            "database",
            database.is_some(),
            &application_type,
        );
        let method_names = methods.iter().map(|method| method.name.clone()).collect();
        let event_names = events.iter().map(|event| event.name.clone()).collect();
        let permissions = market_manifest_permissions(&value.latest_version.manifest);
        Self {
            app_id: value.app_id,
            application_type,
            name: value.name,
            description: value.description,
            source,
            repo: value.latest_version.repo.clone().unwrap_or_default(),
            revision: value.latest_version.revision.clone().unwrap_or_default(),
            checksum,
            archive_path,
            risk: value.risk,
            risk_level: value.risk_level.unwrap_or_else(|| "medium".to_string()),
            capability: value.capability,
            version: value.latest_version.version,
            published_at: value.latest_version.published_at,
            icon_data_url,
            release_notes,
            configuration_declaration,
            interface_declaration,
            database_declaration,
            config_schema,
            database,
            methods,
            events,
            method_names,
            event_names,
            permissions,
            compatible: host_compatibility.compatible,
            compatibility_message: host_compatibility.message,
            minimum_host_version: host_compatibility.minimum_host_version,
            required_host_capabilities: host_compatibility.required_capabilities,
            missing_host_capabilities: host_compatibility.missing_capabilities,
        }
    }
}

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
