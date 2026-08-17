use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AGENT_PROTOCOL_VERSION: u32 = 2;
pub const AGENT_PROTOCOL_FEATURE_REGISTERED_ACK: &str = "registered_ack";
pub const AGENT_PROTOCOL_FEATURE_LOCAL_APP_EVENTS_V1: &str = "local_app_events_v1";
pub const AGENT_PROTOCOL_FEATURE_LOCAL_APP_CAPABILITIES_V2: &str = "local_app_capabilities_v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Capabilities(AgentCapabilities),
    RegisteredAck(RegisteredAck),
    InvokeRequest(InvokeRequest),
    InvokeResult(InvokeResult),
    EventEmitted(EventEmitted),
    LocalAppInvokeRequest(LocalAppInvokeRequest),
    LocalAppInvokeResult(InvokeResult),
    LocalAppEventEmitted(LocalAppEventEmitted),
    EventAck(EventAck),
    Error(ProtocolError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub agent_id: String,
    #[serde(default = "default_agent_protocol_version")]
    pub protocol_version: u32,
    #[serde(default)]
    pub protocol_features: Vec<String>,
    #[serde(default)]
    pub services: Vec<ServiceDefinition>,
    #[serde(default)]
    pub local_apps: Vec<LocalAppDefinition>,
}

fn default_agent_protocol_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredAck {
    pub agent_id: String,
    pub workspace_id: u64,
    pub connection_id: String,
    pub registered_at_epoch_seconds: u64,
    pub heartbeat_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDefinition {
    pub name: String,
    pub description: String,
    pub methods: Vec<MethodDefinition>,
    #[serde(default)]
    pub events: Vec<EventDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default, rename = "responseMode", alias = "response_mode")]
    pub response_mode: ResponseMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseMode {
    #[default]
    Cmodel,
    Plain,
    Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub payload_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAppDefinition {
    #[serde(rename = "connectorId", alias = "connector_id")]
    pub connector_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub methods: Vec<MethodDefinition>,
    #[serde(default)]
    pub events: Vec<EventDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEmitted {
    #[serde(default)]
    pub event_id: Option<String>,
    pub service: String,
    pub event: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAppEventEmitted {
    pub event_id: String,
    pub connector_id: String,
    pub event: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAck {
    pub event_id: String,
    #[serde(default)]
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeRequest {
    pub request_id: String,
    pub service: String,
    pub method: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAppInvokeRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<u64>,
    pub connector_id: String,
    pub method: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeResult {
    pub request_id: String,
    pub success: bool,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub error: Option<InvokeError>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolError {
    pub request_id: Option<String>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn method_response_mode_defaults_to_cmodel_and_serializes_as_camel_case() {
        let defaulted: MethodDefinition = serde_json::from_value(json!({
            "name": "invoke",
            "description": "",
            "input_schema": {}
        }))
        .unwrap();
        assert_eq!(defaulted.response_mode, ResponseMode::Cmodel);

        let serialized = serde_json::to_value(MethodDefinition {
            name: "invoke".to_string(),
            description: String::new(),
            input_schema: json!({}),
            response_mode: ResponseMode::Passthrough,
        })
        .unwrap();
        assert_eq!(serialized["responseMode"], "passthrough");
        assert!(serialized.get("response_mode").is_none());
    }

    #[test]
    fn local_app_definition_accepts_legacy_identity_and_serializes_canonical_identity() {
        let definition: LocalAppDefinition = serde_json::from_value(json!({
            "connector_id": "com.baijimu.connector.test",
            "name": "Test",
            "version": "1.0.0",
            "description": "",
            "methods": [],
            "events": []
        }))
        .unwrap();

        let serialized = serde_json::to_value(definition).unwrap();
        assert_eq!(serialized["connectorId"], "com.baijimu.connector.test");
        assert!(serialized.get("connector_id").is_none());
    }
}
