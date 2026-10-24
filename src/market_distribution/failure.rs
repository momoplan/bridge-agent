//! Public API failure presentation; raw transport diagnostics stay in local logs.
use baijimu_cmodel_core::CModelFailureData;
use baijimu_cmodel_http::CModelHttpError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerFailure {
    pub message: String,
    pub error_code: Option<String>,
    pub retryable: Option<bool>,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub status: Option<u16>,
    pub details: Option<String>,
}

impl ConsumerFailure {
    pub(super) fn response(message: &str, response: &reqwest::Response) -> Self {
        let id = |header| {
            response
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Self::local(
            message,
            id(baijimu_cmodel_http::REQUEST_ID_HEADER),
            id(baijimu_cmodel_http::TRACE_ID_HEADER),
            Some(response.status().as_u16()),
        )
    }

    pub(super) fn local(
        message: &str,
        request_id: Option<String>,
        trace_id: Option<String>,
        status: Option<u16>,
    ) -> Self {
        Self {
            message: message.into(),
            error_code: None,
            retryable: None,
            request_id,
            trace_id,
            status,
            details: None,
        }
    }

    pub(super) fn decode(
        error: CModelHttpError,
        request_id: Option<String>,
        trace_id: Option<String>,
        status: u16,
    ) -> Self {
        tracing::error!(request_id, trace_id, error = %error, "consumer response violates contract");
        Self::local(
            "应用服务返回的数据不符合协议，请联系管理员。",
            request_id,
            trace_id,
            Some(status),
        )
    }

    pub(super) fn downstream(
        code: &str,
        data: Option<&CModelFailureData>,
        status: u16,
        request_id: Option<String>,
        trace_id: Option<String>,
    ) -> Self {
        let value = data.map(|data| serde_json::to_value(data).expect("validated failure data"));
        let id = |key: &str| {
            value
                .as_ref()
                .and_then(|data| data.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let request_id = id("requestId").or(request_id);
        let trace_id = id("traceId").or(trace_id);
        tracing::warn!(
            request_id,
            trace_id,
            error_code = code,
            status,
            "consumer request failed"
        );
        Self {
            message: data
                .map(|data| data.message())
                .unwrap_or("应用服务拒绝了本次请求，请凭错误码联系管理员。")
                .to_owned(),
            error_code: Some(code.to_owned()),
            retryable: data.map(|data| data.retryable()),
            request_id,
            trace_id,
            status: Some(status),
            details: value.map(|value| serde_json::to_string_pretty(&value).expect("JSON value")),
        }
    }
}

impl std::fmt::Display for ConsumerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.message)?;
        if let Some(code) = &self.error_code {
            writeln!(f, "错误码：{code}")?;
        }
        if let Some(retryable) = self.retryable {
            writeln!(
                f,
                "{}",
                if retryable {
                    "可稍后重试"
                } else {
                    "请处理原因后再重试"
                }
            )?;
        }
        if let Some(id) = &self.request_id {
            writeln!(f, "请求 ID：{id}")?;
        }
        if let Some(id) = &self.trace_id {
            writeln!(f, "追踪 ID：{id}")?;
        }
        if let Some(status) = self.status {
            writeln!(f, "HTTP：{status}")?;
        }
        if let Some(details) = &self.details {
            write!(f, "{details}")?;
        }
        Ok(())
    }
}
impl std::error::Error for ConsumerFailure {}

/// Preserve stream diagnostics after the response headers have been accepted.
pub fn artifact_read_error(error: reqwest::Error, response: &reqwest::Response) -> ConsumerFailure {
    let request_id = response
        .headers()
        .get(baijimu_cmodel_http::REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let trace_id = response
        .headers()
        .get(baijimu_cmodel_http::TRACE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    tracing::error!(request_id, trace_id, error = ?error.without_url(), "consumer artifact stream failed");
    ConsumerFailure::local(
        "安装包下载中断，请稍后重试。",
        request_id,
        trace_id,
        Some(response.status().as_u16()),
    )
}
