use super::*;

pub(super) const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

pub(super) async fn download_bounded_artifact(
    mut response: reqwest::Response,
    destination: &Path,
    label: &str,
    expected_size: u64,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<(), String> {
    if expected_size > MAX_ARTIFACT_BYTES {
        return Err(format!("{label}超过 128 MiB"));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("写入{label}失败: {err}"))?;
    }
    let mut file =
        fs::File::create(destination).map_err(|err| format!("写入{label}失败: {err}"))?;
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
    Ok(())
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

    #[tokio::test]
    async fn download_enforces_the_declared_size_and_the_absolute_cap() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("artifact.bin");
        download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "应用制品",
            5,
            None,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"hello");

        let incomplete = download_bounded_artifact(
            artifact_response(b"hello".to_vec()).await,
            &destination,
            "环境应用制品",
            8,
            None,
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
        )
        .await
        .unwrap_err();
        assert!(capped.contains("应用制品超过 128 MiB"), "{capped}");
    }
}
