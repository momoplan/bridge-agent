use super::*;
use sha2::{Digest, Sha256};

pub(super) const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const MISSING_ARTIFACT_CHECKSUM: &str = "冻结版本没有提供当前平台制品的 SHA-256 校验和";

pub(super) struct FrozenArtifactMetadata {
    pub sha256: String,
    pub archive_path: Option<String>,
}

pub(super) fn frozen_artifact_metadata(
    manifest_json: &str,
    platform: &str,
    architecture: &str,
) -> Result<FrozenArtifactMetadata, String> {
    let manifest: Value =
        serde_json::from_str(manifest_json).map_err(|err| format!("冻结清单无效: {err}"))?;
    let entry = manifest
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
        .ok_or(MISSING_ARTIFACT_CHECKSUM)?;
    let checksum = entry
        .get("checksum")
        .and_then(Value::as_str)
        .ok_or(MISSING_ARTIFACT_CHECKSUM)?;
    Ok(FrozenArtifactMetadata {
        sha256: normalize_sha256(checksum)?,
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
    expected_sha256: &str,
) -> Result<(), String> {
    let expected_sha256 = normalize_sha256(expected_sha256)?;
    if expected_size > MAX_ARTIFACT_BYTES {
        return Err(format!("{label}超过 128 MiB"));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("写入{label}失败: {err}"))?;
    }
    let mut file =
        fs::File::create(destination).map_err(|err| format!("写入{label}失败: {err}"))?;
    let mut hasher = Sha256::new();
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
        hasher.update(&chunk);
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
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        return Err(format!("{label} SHA-256 校验失败"));
    }
    Ok(())
}

fn normalize_sha256(value: &str) -> Result<String, String> {
    let hex = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    if hex.len() != 64 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("制品校验和不是 SHA-256".to_string());
    }
    Ok(hex.to_ascii_lowercase())
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

    #[tokio::test]
    async fn download_enforces_size_and_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("artifact.bin");
        download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "应用制品",
            5,
            None,
            HELLO_SHA256,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"hello");

        let mismatch = download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "应用制品",
            5,
            None,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .await
        .unwrap_err();
        assert!(mismatch.contains("应用制品 SHA-256 校验失败"), "{mismatch}");

        let incomplete = download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "环境应用制品",
            8,
            None,
            HELLO_SHA256,
        )
        .await
        .unwrap_err();
        assert!(
            incomplete.contains("环境应用制品下载不完整"),
            "{incomplete}"
        );

        let oversized = download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "工具制品",
            4,
            None,
            HELLO_SHA256,
        )
        .await
        .unwrap_err();
        assert!(
            oversized.contains("工具制品超过冻结版本声明的大小"),
            "{oversized}"
        );

        let capped = download_bounded_artifact(
            artifact_response(Vec::new()).await,
            &destination,
            "应用制品",
            MAX_ARTIFACT_BYTES + 1,
            None,
            HELLO_SHA256,
        )
        .await
        .unwrap_err();
        assert!(capped.contains("应用制品超过 128 MiB"), "{capped}");
    }

    #[test]
    fn frozen_manifest_requires_sha256_for_the_selected_artifact() {
        let manifest = r#"{
            "artifacts": [{
                "platform": "linux",
                "arch": "x86_64",
                "checksum": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "archivePath": "bin/example"
            }]
        }"#;
        let metadata = frozen_artifact_metadata(manifest, "linux", "x86_64").unwrap();
        assert_eq!(
            metadata.sha256,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(metadata.archive_path.as_deref(), Some("bin/example"));

        let missing = r#"{"artifacts":[{"platform":"linux","arch":"x86_64"}]}"#;
        let error = frozen_artifact_metadata(missing, "linux", "x86_64").unwrap_err();
        assert!(error.contains("SHA-256"));
    }
}
