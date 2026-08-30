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
                    message: format!("local endpoint returned CModel failure {error_code}"),
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
        assert!(error.message.contains("HTTP status"));
    }

    #[test]
    fn preserves_valid_cmodel_failure_code() {
        let body = json!({
            "contractVersion": "1.0.0",
            "errorCode": "RESOURCE_NOT_FOUND",
            "data": null
        });
        let bytes = serde_json::to_vec(&body).unwrap();

        let outcome = normalize_cmodel_http_response(StatusCode::NOT_FOUND, &bytes, "test binding");

        assert!(!outcome.success);
        let error = outcome.error.unwrap();
        assert_eq!(error.code, "RESOURCE_NOT_FOUND");
        assert_eq!(
            error.message,
            "local endpoint returned CModel failure RESOURCE_NOT_FOUND"
        );
    }
}
