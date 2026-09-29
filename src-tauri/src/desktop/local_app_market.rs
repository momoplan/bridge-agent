use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MarketConnectorApp {
    pub(super) app_id: String,
    pub(super) install_source: Option<local_app_contract::InstallSource>,
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
    let consumer = market_consumer::market_consumer(config_path).await?;
    consumer
        .listings()
        .await?
        .into_iter()
        .map(market_listing_presentation)
        .collect()
}

pub(super) fn market_listing_presentation(
    listing: local_app_contract::MarketListing,
) -> Result<MarketConnectorApp, String> {
    listing.validate().map_err(|error| error.to_string())?;
    let manifest: Value = serde_json::from_str(listing.frozen_version.content.manifest.as_json())
        .map_err(|error| error.to_string())?;
    let display = &listing.presentation;
    let source = listing.frozen_version.source.clone();
    let selection = local_app_contract::InstallSource::Market {
        listing_id: listing.listing_id,
        version: source.version.clone(),
        source: source.clone(),
    };
    let mut presentation = MarketConnectorApp::from(RawMarketConnectorApp {
        app_id: source.application.app_id.as_str().to_owned(),
        name: display.name.clone(),
        description: display.description.clone(),
        risk: display
            .risk
            .as_ref()
            .map(|risk| risk.description.clone())
            .unwrap_or_default(),
        risk_level: display
            .risk
            .as_ref()
            .map(|risk| risk.level.as_str().to_owned()),
        capability: display.capability.clone().unwrap_or_default(),
        latest_version: RawMarketConnectorVersion {
            version: source.version.to_string(),
            source: String::new(),
            source_type: None,
            repo: None,
            revision: Some(listing.frozen_version.content.source_revision.clone()),
            checksum: None,
            published_at: None,
            manifest,
            compatibility: None,
        },
    });
    // Manifest URLs are authoring metadata, never a consumer download authority.
    presentation.source.clear();
    presentation.checksum = None;
    presentation.archive_path = None;
    presentation.install_source = Some(selection);
    presentation.icon_data_url = display
        .icon
        .as_ref()
        .map(|icon| {
            connector_icon_data_url(&ConnectorIcon {
                media_type: icon.media_type.clone(),
                data: icon.data.clone(),
            })
            .map_err(|error| error.to_string())
        })
        .transpose()?;
    if bridge_agent::market_distribution::select_artifact(
        &listing.frozen_version,
        normalized_platform(),
        std::env::consts::ARCH,
    )
    .map_err(|error| error.to_string())?
    .is_none()
    {
        presentation.compatible = false;
        presentation.compatibility_message = Some("该版本未提供当前平台的安装包".into());
    }
    Ok(presentation)
}

#[tauri::command]
pub(super) async fn show_connector_app(id: String) -> Result<ConnectorInstallRecord, String> {
    show_connector(id.trim()).map_err(|err| err.to_string())
}

pub(super) enum ResolvedConnectorSource {
    Archive {
        path: PathBuf,
        _temp_dir: tempfile::TempDir,
    },
}

impl ResolvedConnectorSource {
    pub(super) fn path(&self) -> &Path {
        match self {
            Self::Archive { path, .. } => path.as_path(),
        }
    }
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
            install_source: None,
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

include!("local_app_market/presentation.rs");
