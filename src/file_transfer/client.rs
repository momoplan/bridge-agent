use crate::config::AgentConfig;
use baijimu_cmodel_http::{decode_required_data, CModelHttpError};
use futures_util::{stream, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TransferError {
    pub code: String,
    pub message: String,
}
impl TransferError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    fn transport(error: reqwest::Error) -> Self {
        Self::new(
            if error.is_timeout() {
                "TRANSFER_TIMEOUT"
            } else {
                "TRANSFER_CONNECTION_FAILED"
            },
            if error.is_timeout() {
                "文件传输连接超时"
            } else {
                "文件传输连接失败"
            },
        )
    }
}
impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for TransferError {}
impl From<std::io::Error> for TransferError {
    fn from(error: std::io::Error) -> Self {
        Self::new("LOCAL_FILE_ERROR", error.to_string())
    }
}
type Result<T> = std::result::Result<T, TransferError>;
#[derive(Clone)]
pub(crate) enum Purpose {
    File,
    #[cfg(any(test, target_os = "macos", windows))]
    Screenshot,
}
impl Purpose {
    fn label(&self) -> &'static str {
        match self {
            Self::File => "file",
            #[cfg(any(test, target_os = "macos", windows))]
            Self::Screenshot => "screenshot",
        }
    }
    fn endpoint(&self, stage: &str) -> String {
        format!("/api/bridge-agent/files/uploads/{stage}")
    }
}
#[derive(Clone)]
pub(crate) struct FileClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
    agent_id: String,
    workspace_id: Option<u64>,
    timeout_secs: u64,
    pub scope: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ticket {
    contract_version: String,
    file_id: String,
    object_key: String,
    upload_url: String,
    method: String,
    headers: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Completion {
    contract_version: String,
    pub file_id: String,
    pub file_name: String,
    pub path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub download_url: String,
    pub expires_at: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileRef {
    pub file_id: String,
    pub path: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}
impl Completion {
    fn validate(&self) -> Result<()> {
        if self.contract_version != "1.0.0"
            || uuid::Uuid::parse_str(&self.file_id).is_err()
            || self.path.is_empty()
            || self.file_name.is_empty()
            || self.mime_type.is_empty()
            || self.download_url.is_empty()
            || self.expires_at.is_empty()
        {
            return Err(TransferError::new(
                "FILE_RESPONSE_INVALID",
                "文件 Owner 返回的引用无效",
            ));
        }
        Ok(())
    }
    pub fn file_ref(&self, sha256: String) -> FileRef {
        FileRef {
            file_id: self.file_id.clone(),
            path: self.path.clone(),
            file_name: self.file_name.clone(),
            mime_type: self.mime_type.clone(),
            size_bytes: self.size_bytes,
            sha256,
        }
    }
}
pub(crate) enum UploadSource {
    File(tokio::fs::File),
    Bytes(Vec<u8>),
}
pub(crate) struct PendingUpload {
    ticket: Ticket,
    purpose: Purpose,
    mime_type: String,
    size_bytes: u64,
    pub sha256: String,
}
pub(crate) struct PendingDownload {
    path: tempfile::TempPath,
    pub file_ref: FileRef,
}
impl PendingDownload {
    pub fn commit(self, target: &Path, overwrite: bool) -> Result<FileRef> {
        if overwrite {
            self.path.persist(target)
        } else {
            self.path.persist_noclobber(target)
        }
        .map_err(|e| TransferError::from(e.error))?;
        Ok(self.file_ref)
    }
}
impl FileClient {
    pub fn from_config(config: &AgentConfig) -> anyhow::Result<Self> {
        let mut base = url::Url::parse(&config.relay.url)?;
        let scheme = match base.scheme() {
            "wss" | "https" => "https",
            "ws" | "http" => "http",
            _ => anyhow::bail!("invalid Relay URL scheme"),
        };
        base.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid Relay URL"))?;
        if base.host_str().is_none() || !base.username().is_empty() || base.password().is_some() {
            anyhow::bail!("invalid Relay URL authority");
        }
        base.set_path("");
        base.set_query(None);
        base.set_fragment(None);
        let scope = format!(
            "{}:{:?}:{:x}",
            config.relay.agent_id,
            config.platform.workspace_id,
            Sha256::digest(config.relay.token.as_bytes())
        );
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base_url: base.to_string().trim_end_matches('/').into(),
            token: config.relay.token.clone(),
            agent_id: config.relay.agent_id.clone(),
            workspace_id: config.platform.workspace_id,
            timeout_secs: config.upload.timeout_secs,
            scope,
        })
    }
    async fn post<T: DeserializeOwned>(&self, path: &str, mut request: Value) -> Result<T> {
        request["contractVersion"] = json!("1.0.0");
        request["agentId"] = json!(self.agent_id);
        request["workspaceId"] = json!(self.workspace_id);
        let response = self
            .client
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(self.timeout_secs))
            .json(&request)
            .send()
            .await
            .map_err(TransferError::transport)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(TransferError::transport)?;
        decode_required_data(status, &bytes, "device-file-transfer").map_err(|e| match e {
            CModelHttpError::Downstream(e) => {
                TransferError::new(e.error_code(), e.message().unwrap_or("文件 Owner 拒绝请求"))
            }
            _ => TransferError::new(
                "FILE_RESPONSE_INVALID",
                format!("文件服务响应无效（HTTP {}）", status.as_u16()),
            ),
        })
    }
    pub async fn prepare_put(
        &self,
        source: UploadSource,
        file_name: &str,
        mime_type: &str,
        size_bytes: u64,
        purpose: Purpose,
        progress: Arc<AtomicU64>,
    ) -> Result<PendingUpload> {
        let body = json!({"fileName":file_name,"mimeType":mime_type,"sizeBytes":size_bytes,"purpose":purpose.label()});
        let ticket: Ticket = self.post(&purpose.endpoint("prepare"), body).await?;
        if ticket.contract_version != "1.0.0"
            || ticket.method != "PUT"
            || ticket.object_key.is_empty()
            || uuid::Uuid::parse_str(&ticket.file_id).is_err()
        {
            return Err(TransferError::new(
                "FILE_RESPONSE_INVALID",
                "文件 Owner 返回的上传票据无效",
            ));
        }
        let sha256 = put_source(
            &self.client,
            &ticket.upload_url,
            &ticket.headers,
            source,
            size_bytes,
            progress,
        )
        .await?;
        Ok(PendingUpload {
            ticket,
            purpose,
            mime_type: mime_type.into(),
            size_bytes,
            sha256,
        })
    }
    pub async fn complete(&self, pending: PendingUpload) -> Result<(Completion, String)> {
        let file: Completion = self.post(&pending.purpose.endpoint("complete"), json!({"fileId":pending.ticket.file_id,"objectKey":pending.ticket.object_key,"purpose":pending.purpose.label()})).await?;
        file.validate()?;
        if file.file_id != pending.ticket.file_id
            || file.size_bytes != pending.size_bytes
            || file.mime_type != pending.mime_type
        {
            return Err(TransferError::new(
                "FILE_RESPONSE_INVALID",
                "文件 Owner 完成结果与上传不符",
            ));
        }
        Ok((file, pending.sha256))
    }
    #[cfg(any(test, target_os = "macos", windows))]
    pub async fn upload(
        &self,
        source: UploadSource,
        file_name: &str,
        mime_type: &str,
        size_bytes: u64,
        purpose: Purpose,
    ) -> Result<(Completion, String)> {
        let pending = self
            .prepare_put(
                source,
                file_name,
                mime_type,
                size_bytes,
                purpose,
                Arc::new(AtomicU64::new(0)),
            )
            .await?;
        self.complete(pending).await
    }
    pub async fn download(
        &self,
        file_id: &str,
        target: &Path,
        expected_sha256: Option<&str>,
        progress: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
    ) -> Result<PendingDownload> {
        let file: Completion = self
            .post(
                "/api/bridge-agent/files/downloads/prepare",
                json!({"fileId":file_id}),
            )
            .await?;
        file.validate()?;
        if file.file_id != file_id {
            return Err(TransferError::new(
                "FILE_RESPONSE_INVALID",
                "文件 Owner 返回了不同文件",
            ));
        }
        total.store(file.size_bytes, Ordering::Relaxed);
        let temporary = tempfile::NamedTempFile::new_in(
            target
                .parent()
                .ok_or_else(|| TransferError::new("INVALID_PATH", "下载路径必须有父目录"))?,
        )?;
        let (handle, path) = temporary.into_parts();
        let mut handle = tokio::fs::File::from_std(handle);
        let response = self
            .client
            .get(&file.download_url)
            .send()
            .await
            .map_err(TransferError::transport)?;
        if !response.status().is_success() {
            return Err(TransferError::new(
                "OBJECT_STORAGE_DOWNLOAD_FAILED",
                format!("对象存储下载失败（HTTP {}）", response.status().as_u16()),
            ));
        }
        let mut chunks = response.bytes_stream();
        let mut hash = Sha256::new();
        let mut count = 0u64;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(TransferError::transport)?;
            count += chunk.len() as u64;
            if count > file.size_bytes {
                return Err(TransferError::new(
                    "FILE_SIZE_MISMATCH",
                    "下载内容超过文件 Owner 声明的大小",
                ));
            }
            handle.write_all(&chunk).await?;
            hash.update(&chunk);
            progress.store(count, Ordering::Relaxed);
        }
        if count != file.size_bytes {
            return Err(TransferError::new("FILE_SIZE_MISMATCH", "下载文件长度不符"));
        }
        let sha256 = format!("{:x}", hash.finalize());
        if expected_sha256.is_some_and(|expected| !expected.eq_ignore_ascii_case(&sha256)) {
            return Err(TransferError::new(
                "FILE_HASH_MISMATCH",
                "下载文件 SHA-256 不符",
            ));
        }
        handle.flush().await?;
        handle.sync_all().await?;
        drop(handle);
        Ok(PendingDownload {
            path,
            file_ref: file.file_ref(sha256),
        })
    }
}
pub(crate) async fn put_source(
    client: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    source: UploadSource,
    size: u64,
    progress: Arc<AtomicU64>,
) -> Result<String> {
    let hash = Arc::new(Mutex::new(Sha256::new()));
    let body = match source {
        UploadSource::Bytes(bytes) => {
            if bytes.len() as u64 != size {
                return Err(TransferError::new("FILE_SIZE_MISMATCH", "上传内容长度不符"));
            }
            hash.lock().unwrap().update(&bytes);
            progress.store(size, Ordering::Relaxed);
            reqwest::Body::from(bytes)
        }
        UploadSource::File(file) => {
            let hash = hash.clone();
            let progress = progress.clone();
            reqwest::Body::wrap_stream(stream::try_unfold(
                (file, hash, progress),
                |(mut file, hash, progress)| async move {
                    let mut bytes = vec![0u8; 64 * 1024];
                    let n = file.read(&mut bytes).await?;
                    if n == 0 {
                        return Ok::<_, std::io::Error>(None);
                    }
                    bytes.truncate(n);
                    hash.lock().unwrap().update(&bytes);
                    progress.fetch_add(n as u64, Ordering::Relaxed);
                    Ok(Some((bytes, (file, hash, progress))))
                },
            ))
        }
    };
    let mut request = client.put(url).body(body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request
        .header(reqwest::header::CONTENT_LENGTH, size)
        .send()
        .await
        .map_err(TransferError::transport)?;
    if !response.status().is_success() {
        return Err(TransferError::new(
            "OBJECT_STORAGE_UPLOAD_FAILED",
            format!("对象存储上传失败（HTTP {}）", response.status().as_u16()),
        ));
    }
    if progress.load(Ordering::Relaxed) != size {
        return Err(TransferError::new(
            "FILE_SIZE_MISMATCH",
            "上传过程中文件大小发生变化",
        ));
    }
    let digest = format!("{:x}", hash.lock().unwrap().clone().finalize());
    Ok(digest)
}
