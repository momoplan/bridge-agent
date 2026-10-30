use super::client::{FileClient, FileRef, Purpose, TransferError, UploadSource};
use crate::config::AgentConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};
use tokio::sync::{oneshot, Mutex};

type Entries = Arc<Mutex<BTreeMap<(String, String), Entry>>>;
static ENTRIES: OnceLock<Entries> = OnceLock::new();
pub(crate) struct FileMethod {
    client: FileClient,
    method: String,
    entries: Entries,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    transfer_id: String,
    direction: &'static str,
    status: &'static str,
    bytes_transferred: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
    local_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_ref: Option<FileRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<TransferError>,
}
struct Entry {
    snapshot: Snapshot,
    progress: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Option<oneshot::Sender<()>>,
    cancel_requested: bool,
}
impl Entry {
    fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.snapshot.clone();
        snapshot.bytes_transferred = self.progress.load(Ordering::Relaxed);
        snapshot.total_bytes = match self.total.load(Ordering::Relaxed) {
            u64::MAX => None,
            total => Some(total),
        };
        snapshot
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Upload {
    local_path: String,
    file_name: Option<String>,
    mime_type: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Download {
    file_id: String,
    local_path: String,
    #[serde(default)]
    overwrite: bool,
    expected_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferId {
    transfer_id: String,
}
enum Operation {
    Upload(Upload),
    Download(Download),
}
enum Ready {
    Upload(super::client::PendingUpload),
    Download(super::client::PendingDownload, PathBuf, bool),
}
impl FileMethod {
    pub(crate) fn new(config: &AgentConfig, method: &str) -> anyhow::Result<Self> {
        Ok(Self {
            client: FileClient::from_config(config)?,
            method: method.into(),
            entries: ENTRIES
                .get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())))
                .clone(),
        })
    }
    pub(crate) async fn invoke(&self, arguments: Value) -> anyhow::Result<Value> {
        match self.method.as_str() {
            "upload" => {
                let args: Upload = serde_json::from_value(arguments)?;
                absolute_path(&args.local_path)?;
                if args
                    .file_name
                    .as_ref()
                    .is_some_and(|name| name.is_empty() || name.contains(['/', '\\']))
                {
                    anyhow::bail!("fileName must be a file name");
                }
                self.start(Operation::Upload(args)).await
            }
            "download" => {
                let mut args: Download = serde_json::from_value(arguments)?;
                absolute_path(&args.local_path)?;
                args.file_id = uuid::Uuid::parse_str(&args.file_id)?.to_string();
                if args.expected_sha256.as_ref().is_some_and(|value| {
                    value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit())
                }) {
                    anyhow::bail!("expectedSha256 must be a 64-digit hexadecimal SHA-256");
                }
                self.start(Operation::Download(args)).await
            }
            "queryTransfer" | "cancelTransfer" => {
                let args: TransferId = serde_json::from_value(arguments)?;
                let mut entries = self.entries.lock().await;
                let entry = entries
                    .get_mut(&(self.client.scope.clone(), args.transfer_id))
                    .ok_or_else(|| anyhow::anyhow!("unknown file transfer"))?;
                if self.method == "cancelTransfer" {
                    if let Some(cancel) = entry.cancel.take() {
                        entry.cancel_requested = true;
                        let _ = cancel.send(());
                    }
                }
                Ok(serde_json::to_value(entry.snapshot())?)
            }
            _ => anyhow::bail!("unknown files method"),
        }
    }
    async fn start(&self, operation: Operation) -> anyhow::Result<Value> {
        let (direction, path) = match &operation {
            Operation::Upload(args) => ("UPLOAD", &args.local_path),
            Operation::Download(args) => ("DOWNLOAD", &args.local_path),
        };
        let snapshot = Snapshot {
            transfer_id: uuid::Uuid::new_v4().to_string(),
            direction,
            status: "QUEUED",
            bytes_transferred: 0,
            total_bytes: None,
            local_path: path.clone(),
            file_ref: None,
            error: None,
        };
        let key = (self.client.scope.clone(), snapshot.transfer_id.clone());
        let progress = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(u64::MAX));
        let (cancel, receiver) = oneshot::channel();
        self.entries.lock().await.insert(
            key.clone(),
            Entry {
                snapshot: snapshot.clone(),
                progress: progress.clone(),
                total: total.clone(),
                cancel: Some(cancel),
                cancel_requested: false,
            },
        );
        let entries = self.entries.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            entries.lock().await.get_mut(&key).unwrap().snapshot.status = "RUNNING";
            let prepared = tokio::select! {
                biased;
                result = prepare(&client, operation, progress, total) => result,
                _ = receiver => Err(TransferError::new("TRANSFER_CANCELLED", "文件传输已取消")),
            };
            // Cancel can interrupt bytes, but not the Owner's commit or the atomic
            // local rename. Once committing, report RUNNING until the real result.
            let cancelled = {
                let mut entries = entries.lock().await;
                let entry = entries.get_mut(&key).unwrap();
                let cancelled = entry.cancel_requested;
                entry.cancel = None;
                cancelled
            };
            let result = if cancelled {
                drop(prepared);
                Err(TransferError::new("TRANSFER_CANCELLED", "文件传输已取消"))
            } else {
                match prepared {
                    Ok(Ready::Upload(upload)) => client
                        .complete(upload)
                        .await
                        .map(|(file, sha256)| file.file_ref(sha256)),
                    Ok(Ready::Download(download, target, overwrite)) => {
                        download.commit(&target, overwrite)
                    }
                    Err(error) => Err(error),
                }
            };
            let mut entries = entries.lock().await;
            let entry = entries.get_mut(&key).unwrap();
            match result {
                Ok(file) => {
                    entry.snapshot.status = "SUCCEEDED";
                    entry.snapshot.file_ref = Some(file);
                }
                Err(error) => {
                    entry.snapshot.status = if error.code == "TRANSFER_CANCELLED" {
                        "CANCELLED"
                    } else {
                        "FAILED"
                    };
                    entry.snapshot.error = Some(error);
                }
            }
        });
        Ok(serde_json::to_value(snapshot)?)
    }
}
fn absolute_path(path: &str) -> anyhow::Result<()> {
    if !PathBuf::from(path).is_absolute() {
        anyhow::bail!("localPath must be an absolute file path");
    }
    Ok(())
}
async fn prepare(
    client: &FileClient,
    operation: Operation,
    progress: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
) -> Result<Ready, TransferError> {
    match operation {
        Operation::Upload(args) => {
            let file = tokio::fs::File::open(&args.local_path).await?;
            let metadata = file.metadata().await?;
            if !metadata.is_file() {
                return Err(TransferError::new(
                    "INVALID_PATH",
                    "localPath 必须是普通文件",
                ));
            }
            total.store(metadata.len(), Ordering::Relaxed);
            let path = PathBuf::from(&args.local_path);
            let name = args
                .file_name
                .as_deref()
                .or_else(|| path.file_name().and_then(|name| name.to_str()))
                .ok_or_else(|| TransferError::new("INVALID_PATH", "本地文件名无效"))?;
            client
                .prepare_put(
                    UploadSource::File(file),
                    name,
                    args.mime_type
                        .as_deref()
                        .unwrap_or("application/octet-stream"),
                    metadata.len(),
                    Purpose::File,
                    progress,
                )
                .await
                .map(Ready::Upload)
        }
        Operation::Download(args) => {
            let target = PathBuf::from(args.local_path);
            client
                .download(
                    &args.file_id,
                    &target,
                    args.expected_sha256.as_deref(),
                    progress,
                    total,
                )
                .await
                .map(|pending| Ready::Download(pending, target, args.overwrite))
        }
    }
}
