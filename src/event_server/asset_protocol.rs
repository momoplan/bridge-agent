//! Versioned Connector upload protocol. Only committed owner files become asset references.
use super::*;
use baijimu_cmodel_http::{decode_required_data, CModelHttpError};
use serde::de::DeserializeOwned;

#[derive(Debug, Deserialize, Serialize)]
pub(super) enum ContractVersion {
    #[serde(rename = "1.0.0")]
    Current,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssetUploadError {
    #[serde(skip)]
    status: StatusCode,
    error_code: String,
    message: String,
    retryable: Option<bool>,
    stage: &'static str,
}
impl AssetUploadError {
    pub(super) fn new(
        status: StatusCode,
        code: &str,
        message: impl Into<String>,
        retryable: bool,
        stage: &'static str,
    ) -> Self {
        Self {
            status,
            error_code: code.into(),
            message: message.into(),
            retryable: Some(retryable),
            stage,
        }
    }
    pub(super) fn transport(error: reqwest::Error, stage: &'static str) -> Self {
        if error.is_timeout() {
            Self::new(
                StatusCode::GATEWAY_TIMEOUT,
                "TIMEOUT",
                "截图上传请求超时",
                true,
                stage,
            )
        } else {
            Self::new(
                StatusCode::BAD_GATEWAY,
                "BAD_GATEWAY",
                "截图上传连接失败",
                true,
                stage,
            )
        }
    }
    fn decode<T: DeserializeOwned>(
        status: StatusCode,
        bytes: &[u8],
        stage: &'static str,
    ) -> Result<T, Self> {
        if !status.is_success() {
            return Err(Self::new(
                StatusCode::BAD_GATEWAY,
                "BAD_GATEWAY",
                format!("上传上游返回 HTTP {}", status.as_u16()),
                status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
                stage,
            ));
        }
        decode_required_data(status, bytes, stage).map_err(|error| match error {
            CModelHttpError::Downstream(error) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                error_code: error.error_code().into(),
                message: error
                    .message()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("上传服务拒绝请求：{}", error.error_code())),
                retryable: error.retryable(),
                stage,
            },
            _ => Self::new(
                StatusCode::BAD_GATEWAY,
                "UPLOAD_RESPONSE_INVALID",
                "上传服务响应不符合合同",
                false,
                stage,
            ),
        })
    }
}
impl From<EventApiError> for AssetUploadError {
    fn from(error: EventApiError) -> Self {
        Self::new(
            error.status,
            "LOCAL_ASSET_REQUEST_FAILED",
            error.message,
            false,
            "validate",
        )
    }
}
impl IntoResponse for AssetUploadError {
    fn into_response(self) -> Response {
        // Keep the existing `error` text while exposing the original failure identity.
        #[derive(Serialize)]
        struct Failure<'a> {
            error: &'a str,
            #[serde(flatten)]
            details: &'a AssetUploadError,
        }
        (
            self.status,
            Json(Failure {
                error: &self.message,
                details: &self,
            }),
        )
            .into_response()
    }
}

pub(super) fn base_url(relay: &str) -> Option<String> {
    let mut url = url::Url::parse(relay).ok()?;
    let scheme = match url.scheme() {
        "wss" | "https" => "https",
        "ws" | "http" => "http",
        _ => return None,
    };
    url.set_scheme(scheme).ok()?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.set_path("/api/bridge-agent/local-app-assets/");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

pub(super) async fn post<I: Serialize, O: DeserializeOwned>(
    state: &EventServerState,
    stage: &'static str,
    request: &I,
) -> Result<O, AssetUploadError> {
    let base = state.asset_base_url.as_deref().ok_or_else(|| {
        AssetUploadError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "UPLOAD_NOT_CONFIGURED",
            "设备 Relay 地址配置无效",
            false,
            stage,
        )
    })?;
    let response = state
        .http_client
        .post(format!("{base}{stage}"))
        .timeout(Duration::from_secs(state.upload_timeout_secs))
        .bearer_auth(&state.relay_token)
        .json(request)
        .send()
        .await
        .map_err(|error| AssetUploadError::transport(error, stage))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AssetUploadError::transport(error, stage))?;
    AssetUploadError::decode(status, &bytes, stage)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteAsset<'a> {
    contract_version: ContractVersion,
    agent_id: &'a str,
    workspace_id: Option<u64>,
    file_id: &'a str,
    object_key: &'a str,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CompletedAsset {
    contract_version: ContractVersion,
    pub file_id: String,
    file_name: String,
    pub path: String,
    mime_type: String,
    size_bytes: u64,
    pub download_url: String,
    pub expires_at: String,
}
pub(super) async fn complete(
    state: &EventServerState,
    slot: &PrepareAssetUploadResponse,
    asset: &ValidatedLocalAsset,
) -> Result<CompletedAsset, AssetUploadError> {
    let result: CompletedAsset = post(
        state,
        "complete",
        &CompleteAsset {
            contract_version: ContractVersion::Current,
            agent_id: &state.agent_id,
            workspace_id: state.workspace_id,
            file_id: &slot.file_id,
            object_key: &slot.object_key,
        },
    )
    .await?;
    let _version = &result.contract_version;
    if result.file_id != slot.file_id
        || result.mime_type != asset.content_type
        || result.size_bytes != asset.size_bytes
        || result.file_name.is_empty()
        || result.path.is_empty()
        || result.download_url.is_empty()
        || result.expires_at.is_empty()
    {
        return Err(AssetUploadError::new(
            StatusCode::BAD_GATEWAY,
            "UPLOAD_RESPONSE_INVALID",
            "已完成文件与本次上传不符",
            false,
            "complete",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
