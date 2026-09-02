use crate::protocol::InvokeError;
use baijimu_cmodel_core::ErrorCode;
use baijimu_cmodel_http::{decode_optional_data, CModelHttpError};
use reqwest::StatusCode;
use serde_json::Value;

pub(crate) struct CModelHttpOutcome {
    pub(crate) success: bool,
    pub(crate) data: Option<Value>,
    pub(crate) error: Option<InvokeError>,
}

pub(crate) fn normalize_cmodel_http_response(
    status: StatusCode,
    bytes: &[u8],
    operation: &str,
) -> CModelHttpOutcome {
    match decode_optional_data::<Value>(status, bytes, operation) {
        Ok(data) => CModelHttpOutcome {
            success: true,
            data,
            error: None,
        },
        Err(CModelHttpError::Downstream(error)) => {
            let error_code = ErrorCode::parse(error.error_code())
                .expect("shared CModel decoder must return a validated error code");
            CModelHttpOutcome {
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: error_code.as_str().to_string(),
                    message: error.message().map(str::to_owned).unwrap_or_else(|| {
                        format!("local endpoint returned CModel failure {error_code}")
                    }),
                }),
            }
        }
        Err(error) => CModelHttpOutcome {
            success: false,
            data: None,
            error: Some(InvokeError {
                code: "HTTP_RESPONSE_INVALID".to_string(),
                message: error.to_string(),
            }),
        },
    }
}

pub fn describe_cmodel_http_outcome(
    status: StatusCode,
    bytes: &[u8],
    operation: &str,
) -> Option<String> {
    let body = serde_json::from_slice::<Value>(bytes).ok()?;
    if body.get("errorCode").is_none() && body.get("contractVersion").is_none() {
        return None;
    }

    Some(
        match decode_optional_data::<Value>(status, bytes, operation) {
            Ok(_) => format!("HTTP {status}: CModel 响应表示业务成功"),
            Err(CModelHttpError::Downstream(error)) => match error.message() {
                Some(message) => format!("HTTP {status}: {}: {message}", error.error_code()),
                None => format!("HTTP {status}: {}", error.error_code()),
            },
            Err(error) => format!("HTTP {status}: CModel 协议错误: {error}"),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_contract_invalid_status_and_body() {
        let body = json!({
            "contractVersion": "1.0.0",
            "errorCode": "0",
            "data": {"ok": true}
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let outcome =
            normalize_cmodel_http_response(StatusCode::BAD_GATEWAY, &bytes, "test binding");

        assert!(!outcome.success);
        let error = outcome.error.unwrap();
        assert_eq!(error.code, "HTTP_RESPONSE_INVALID");
        assert!(error.message.contains("must use HTTP 200"));
    }

    #[test]
    fn preserves_standard_cmodel_failure_details() {
        let body = json!({
            "contractVersion": "1.0.0",
            "errorCode": "CUSTOM_CONNECTOR_FAILURE",
            "data": {
                "message": "资源不存在",
                "retryable": false
            }
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let outcome = normalize_cmodel_http_response(StatusCode::OK, &bytes, "test binding");

        assert!(!outcome.success);
        let error = outcome.error.unwrap();
        assert_eq!(error.code, "CUSTOM_CONNECTOR_FAILURE");
        assert_eq!(error.message, "资源不存在");
    }

    #[test]
    fn uses_stable_fallback_for_failure_without_public_details() {
        let body = json!({
            "contractVersion": "1.0.0",
            "errorCode": "RESOURCE_NOT_FOUND",
            "value": "不能继续读取的遗留字段",
            "data": null
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let outcome = normalize_cmodel_http_response(StatusCode::OK, &bytes, "test binding");

        let error = outcome.error.unwrap();
        assert_eq!(error.code, "RESOURCE_NOT_FOUND");
        assert_eq!(
            error.message,
            "local endpoint returned CModel failure RESOURCE_NOT_FOUND"
        );
    }

    #[test]
    fn accepts_legacy_non_200_failure_during_consumer_migration() {
        let body = json!({
            "contractVersion": "1.0.0",
            "errorCode": "RESOURCE_NOT_FOUND",
            "data": null
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let outcome = normalize_cmodel_http_response(StatusCode::NOT_FOUND, &bytes, "test binding");

        let error = outcome.error.unwrap();
        assert_eq!(error.code, "RESOURCE_NOT_FOUND");
    }

    #[test]
    fn describes_standard_failure_without_reading_legacy_fields() {
        let body = json!({
            "contractVersion": "1.0.0",
            "errorCode": "PAYMENT_REQUIRED",
            "value": "不能展示的遗留字段",
            "data": {
                "message": "余额不足",
                "retryable": false
            }
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let description =
            describe_cmodel_http_outcome(StatusCode::BAD_REQUEST, &bytes, "platform authorization")
                .unwrap();

        assert_eq!(
            description,
            "HTTP 400 Bad Request: PAYMENT_REQUIRED: 余额不足"
        );
        assert!(!description.contains("遗留字段"));
    }

    #[test]
    fn reports_malformed_cmodel_candidate_as_protocol_error() {
        let body = r#"{"contractVersion":"1.0.0","errorCode":"INVALID","data":{"message":"缺少 retryable"}}"#;

        let description = describe_cmodel_http_outcome(
            StatusCode::BAD_REQUEST,
            body.as_bytes(),
            "platform authorization",
        )
        .unwrap();

        assert!(description.contains("CModel 协议错误"));
        assert!(description.contains("retryable"));
    }

    #[test]
    fn ignores_non_cmodel_json() {
        assert!(describe_cmodel_http_outcome(
            StatusCode::BAD_REQUEST,
            br#"{"message":"ordinary error"}"#,
            "platform authorization",
        )
        .is_none());
    }
}
