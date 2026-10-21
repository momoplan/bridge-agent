use super::*;
use sha2::{Digest, Sha256};

pub(super) const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

pub(super) struct FrozenArtifactMetadata {
    pub sha256: Option<String>,
    pub archive_path: Option<String>,
}

/// Read an optional SHA-256 from the frozen manifest. The market `Artifact`
/// type has no digest, and a manifest `checksum` string is not guaranteed to
/// be SHA-256. Anything else is ignored so the download can continue with a
/// size check.
pub(super) fn frozen_artifact_metadata(
    manifest_json: &str,
    platform: &str,
    architecture: &str,
) -> Result<FrozenArtifactMetadata, String> {
    let manifest: Value =
        serde_json::from_str(manifest_json).map_err(|err| format!("冻结清单无效: {err}"))?;
    let Some(entry) = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .and_then(|artifacts| {
            artifacts.iter().find(|entry| {
                entry.get("platform").and_then(Value::as_str) == Some(platform)
                    && entry
                        .get("arch")
                        .and_then(Value::as_str)
                        .unwrap_or("universal")
                        == architecture
            })
        })
    else {
        return Ok(FrozenArtifactMetadata {
            sha256: None,
            archive_path: None,
        });
    };
    Ok(FrozenArtifactMetadata {
        sha256: entry
            .get("checksum")
            .and_then(Value::as_str)
            .and_then(recognized_sha256),
        archive_path: entry
            .get("archivePath")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub(super) async fn download_bounded_artifact(
    mut response: reqwest::Response,
    destination: &Path,
    label: &str,
    expected_size: u64,
    progress: Option<&LocalAppInstallProgressReporter>,
    checksum: Option<&str>,
) -> Result<(), String> {
    let expected_sha256 = checksum.and_then(recognized_sha256);
    if expected_sha256.is_none() {
        log::info!("{label} digest verification skipped; checksum is absent or not a SHA-256");
    }
    if expected_size > MAX_ARTIFACT_BYTES {
        return Err(format!("{label}超过 128 MiB"));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("写入{label}失败: {err}"))?;
    }
    let mut file =
        fs::File::create(destination).map_err(|err| format!("写入{label}失败: {err}"))?;
    let mut hasher = expected_sha256.as_ref().map(|_| Sha256::new());
    let mut downloaded = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("读取{label}失败"))?
    {
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .filter(|size| *size <= expected_size)
            .ok_or_else(|| format!("{label}超过冻结版本声明的大小"))?;
        if let Some(hasher) = hasher.as_mut() {
            hasher.update(&chunk);
        }
        file.write_all(&chunk)
            .map_err(|err| format!("写入{label}失败: {err}"))?;
        if let Some(progress) = progress {
            progress.download(downloaded, Some(expected_size));
        }
    }
    file.sync_all()
        .map_err(|err| format!("写入{label}失败: {err}"))?;
    if downloaded != expected_size {
        return Err(format!("{label}下载不完整"));
    }
    reject_digest_mismatch(hasher, expected_sha256.as_deref(), label)
}

fn reject_digest_mismatch(
    hasher: Option<Sha256>,
    expected: Option<&str>,
    label: &str,
) -> Result<(), String> {
    let (Some(hasher), Some(expected)) = (hasher, expected) else {
        return Ok(());
    };
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(format!("{label} SHA-256 校验失败"));
    }
    Ok(())
}

fn recognized_sha256(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let hex = trimmed
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("sha256:"))
        .map(|_| &trimmed[7..])
        .unwrap_or(trimmed)
        .trim();
    if hex.len() == 64 && hex.chars().all(|character| character.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn artifact_response(body: Vec<u8>) -> reqwest::Response {
        let body = Arc::new(body);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = Arc::clone(&body);
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/artifact",
                    get(move || {
                        let served = Arc::clone(&served);
                        async move { served.as_ref().clone() }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        reqwest::Client::new()
            .get(format!("http://{addr}/artifact"))
            .send()
            .await
            .unwrap()
    }

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    async fn download_hello(checksum: Option<&str>, expected_size: u64) -> Result<Vec<u8>, String> {
        let dir = tempfile::tempdir().map_err(|err| err.to_string())?;
        let destination = dir.path().join("artifact.bin");
        download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "应用制品",
            expected_size,
            None,
            checksum,
        )
        .await?;
        fs::read(&destination).map_err(|err| err.to_string())
    }

    #[tokio::test]
    async fn sha256_checksum_matches() {
        let bytes = download_hello(Some(HELLO_SHA256), 5).await.unwrap();
        assert_eq!(bytes, b"hello");
        let prefixed_checksum = format!("sha256:{HELLO_SHA256}");
        let prefixed = download_hello(Some(&prefixed_checksum), 5).await.unwrap();
        assert_eq!(prefixed, b"hello");
    }

    #[tokio::test]
    async fn sha256_checksum_mismatch_fails() {
        let error = download_hello(
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            5,
        )
        .await
        .unwrap_err();
        assert!(error.contains("SHA-256 校验失败"), "{error}");
    }

    #[tokio::test]
    async fn absent_or_unrecognized_checksum_keeps_the_size_check() {
        let absent = download_hello(None, 5).await.unwrap();
        assert_eq!(absent, b"hello");
        let opaque = download_hello(Some("opaque-market-checksum"), 5)
            .await
            .unwrap();
        assert_eq!(opaque, b"hello");

        let mismatch = download_hello(None, 4).await.unwrap_err();
        assert!(mismatch.contains("超过冻结版本声明的大小"), "{mismatch}");
        let incomplete = download_hello(None, 8).await.unwrap_err();
        assert!(incomplete.contains("下载不完整"), "{incomplete}");
    }

    #[test]
    fn frozen_manifest_uses_sha256_only_when_recognizable() {
        let with_digest = r#"{
            "artifacts": [{
                "platform": "linux",
                "arch": "x86_64",
                "checksum": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "archivePath": "bin/example"
            }]
        }"#;
        let metadata = frozen_artifact_metadata(with_digest, "linux", "x86_64").unwrap();
        let expected = "a".repeat(64);
        assert_eq!(metadata.sha256.as_deref(), Some(expected.as_str()));
        assert_eq!(metadata.archive_path.as_deref(), Some("bin/example"));

        let missing =
            r#"{"artifacts":[{"platform":"linux","arch":"x86_64","archivePath":"bin/example"}]}"#;
        let metadata = frozen_artifact_metadata(missing, "linux", "x86_64").unwrap();
        assert!(metadata.sha256.is_none());
        assert_eq!(metadata.archive_path.as_deref(), Some("bin/example"));

        let opaque = r#"{"artifacts":[{"platform":"linux","arch":"x86_64","checksum":"opaque-market-checksum"}]}"#;
        let metadata = frozen_artifact_metadata(opaque, "linux", "x86_64").unwrap();
        assert!(metadata.sha256.is_none());
    }
}
