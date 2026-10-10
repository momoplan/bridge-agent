//! Relay upload preparation returns a bare upload slot on success and a CModel
//! failure on rejection. Never interpret a failure envelope as an upload slot.
use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;

pub(crate) fn decode_upload_slot<T: DeserializeOwned>(
    status: StatusCode,
    bytes: &[u8],
) -> Result<T> {
    if let Some(details) =
        crate::cmodel_response::describe_cmodel_http_outcome(status, bytes, "prepare upload")
    {
        bail!("prepare upload rejected: {details}");
    }
    if !status.is_success() {
        bail!("prepare upload failed with HTTP {status}");
    }
    serde_json::from_slice(bytes).context("invalid prepare upload slot response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Slot {
        file_id: String,
        upload_url: String,
    }

    #[test]
    fn accepts_the_declared_bare_upload_slot() {
        let slot: Slot = decode_upload_slot(
            StatusCode::OK,
            br#"{"fileId":"screenshot.png","uploadUrl":"https://upload.example.test/object"}"#,
        )
        .unwrap();
        assert_eq!(slot.file_id, "screenshot.png");
        assert_eq!(slot.upload_url, "https://upload.example.test/object");
    }

    #[test]
    fn http_200_business_failure_keeps_public_error_details() {
        let error = decode_upload_slot::<Slot>(StatusCode::OK,
            br#"{"contractVersion":"1.0.0","errorCode":"SERVICE_UNAVAILABLE","data":{"message":"upload backend unavailable","retryable":true}}"#).unwrap_err().to_string();
        assert!(error.contains("SERVICE_UNAVAILABLE"));
        assert!(error.contains("upload backend unavailable"));
    }

    #[test]
    fn rejects_enveloped_success_instead_of_accepting_another_protocol() {
        let error = decode_upload_slot::<Slot>(StatusCode::OK,
            br#"{"contractVersion":"1.0.0","errorCode":"0","data":{"fileId":"a","uploadUrl":"https://upload.example.test"}}"#).unwrap_err();
        assert!(error.to_string().contains("prepare upload rejected"));
    }

    #[test]
    fn rejects_http_failure_without_printing_response_secrets() {
        let error =
            decode_upload_slot::<Slot>(StatusCode::BAD_GATEWAY, b"proxy failed: private-token")
                .unwrap_err()
                .to_string();
        assert!(error.contains("502"));
        assert!(!error.contains("private-token"));
    }

    #[test]
    fn malformed_success_is_a_slot_error() {
        let error = decode_upload_slot::<Slot>(StatusCode::OK, b"{}").unwrap_err();
        assert!(format!("{error:#}").contains("fileId"));
    }
}
