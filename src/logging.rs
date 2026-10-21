use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;

const LOG_QUEUE_CAPACITY: usize = 256;

enum LogCommand {
    Append(String),
    Flush(mpsc::Sender<()>),
    Clear(mpsc::Sender<std::result::Result<(), String>>),
}

#[derive(Debug, Clone)]
pub struct FileLogConfig {
    pub enabled: bool,
    pub dir: Option<PathBuf>,
    pub max_bytes: u64,
    pub max_files: usize,
}

#[derive(Debug, Clone)]
pub struct FileLogSink {
    path: PathBuf,
    tx: SyncSender<LogCommand>,
    dropped: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LogMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
}

impl LogMetadata {
    pub fn category(category: impl Into<String>) -> Self {
        Self {
            category: Some(category.into()),
            ..Self::default()
        }
    }

    pub fn service(mut self, service: impl Into<String>) -> Self {
        self.service = Some(service.into());
        self
    }

    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }

    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn event_id(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }

    pub fn outcome(mut self, outcome: impl Into<String>) -> Self {
        self.outcome = Some(outcome.into());
        self
    }

    pub fn duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    pub fn http(mut self, method: impl Into<String>, path: impl Into<String>, status: u16) -> Self {
        self.http_method = Some(method.into());
        self.path = Some(path.into());
        self.status_code = Some(status);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
    #[serde(flatten)]
    pub metadata: LogMetadata,
}

impl FileLogSink {
    pub fn from_config(config: &FileLogConfig, config_base_dir: &Path) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }

        let dir = match &config.dir {
            Some(dir) if dir.is_absolute() => dir.clone(),
            Some(dir) => config_base_dir.join(dir),
            None => default_log_dir().context("failed to determine log directory")?,
        };

        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create log directory {}", dir.display()))?;

        let path = dir.join("bridge-agent.log");
        let max_bytes = config.max_bytes.max(1024);
        let max_files = config.max_files.max(1);
        let (tx, rx) = mpsc::sync_channel(LOG_QUEUE_CAPACITY);
        let writer_path = path.clone();
        thread::Builder::new()
            .name("bridge-agent-log".to_string())
            .spawn(move || log_writer(writer_path, max_bytes, max_files, rx))
            .context("failed to start log writer")?;
        Ok(Some(Self {
            path,
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &LogEntry) -> Result<()> {
        let line = serde_json::to_string(entry)?;
        match self.tx.try_send(LogCommand::Append(line)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("log writer stopped")
            }
        }
    }

    pub fn flush(&self) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.tx
            .send(LogCommand::Flush(ack_tx))
            .context("log writer stopped")?;
        ack_rx.recv().context("log writer stopped while flushing")
    }

    pub fn clear(&self) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::channel();
        self.tx
            .send(LogCommand::Clear(ack_tx))
            .context("log writer stopped")?;
        ack_rx
            .recv()
            .context("log writer stopped while clearing")?
            .map_err(|err| anyhow::anyhow!(err))
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

struct LogWriter {
    path: PathBuf,
    max_bytes: u64,
    max_files: usize,
    file: Option<BufWriter<std::fs::File>>,
    written: u64,
}

impl LogWriter {
    fn append_line(&mut self, line: &str) -> Result<()> {
        let next = self
            .written
            .saturating_add(line.len() as u64)
            .saturating_add(1);
        if self.file.is_some() && self.written > 0 && next > self.max_bytes {
            self.rotate()?;
        }
        self.ensure_open()?;
        let file = self
            .file
            .as_mut()
            .context("log file is not open after rotation")?;
        writeln!(file, "{line}")
            .with_context(|| format!("failed to write log file {}", self.path.display()))?;
        self.written = self
            .written
            .saturating_add(line.len() as u64)
            .saturating_add(1);
        Ok(())
    }

    fn ensure_open(&mut self) -> Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create log directory {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open log file {}", self.path.display()))?;
        self.written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        self.file = Some(BufWriter::new(file));
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
                .with_context(|| format!("failed to flush log file {}", self.path.display()))?;
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.flush()?;
        self.file = None;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create log directory {}", parent.display()))?;
        }
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)
            .with_context(|| format!("failed to clear log file {}", self.path.display()))?;
        self.written = 0;
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        self.flush()?;
        self.file = None;
        for index in (1..=self.max_files).rev() {
            let from = rotated_path(&self.path, index);
            let to = rotated_path(&self.path, index + 1);
            if index == self.max_files {
                let _ = fs::remove_file(&from);
            } else if from.exists() {
                let _ = fs::rename(&from, &to);
            }
        }
        let first = rotated_path(&self.path, 1);
        let _ = fs::rename(&self.path, &first);
        self.written = 0;
        Ok(())
    }
}

fn log_writer(path: PathBuf, max_bytes: u64, max_files: usize, rx: Receiver<LogCommand>) {
    let mut writer = LogWriter {
        path,
        max_bytes,
        max_files,
        file: None,
        written: 0,
    };
    while let Ok(command) = rx.recv() {
        match command {
            LogCommand::Append(line) => {
                if let Err(err) = writer.append_line(&line) {
                    eprintln!("failed to append file log: {err:#}");
                }
            }
            LogCommand::Flush(ack) => {
                let _ = writer.flush();
                let _ = ack.send(());
            }
            LogCommand::Clear(ack) => {
                let result = writer.clear().map_err(|err| err.to_string());
                let _ = ack.send(result);
            }
        }
    }
    let _ = writer.flush();
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.display(), index))
}

fn default_log_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(program_data) = std::env::var_os("ProgramData") {
            return Some(
                PathBuf::from(program_data)
                    .join("Baijimu")
                    .join("BridgeAgent")
                    .join("logs"),
            );
        }
    }

    ProjectDirs::from("com", "baijimu", "bridge-agent")
        .map(|dirs| dirs.data_local_dir().join("logs"))
}

#[cfg(test)]
mod tests {
    use super::{FileLogConfig, FileLogSink};
    use tempfile::tempdir;

    #[test]
    fn file_log_sink_writes_json_lines() {
        let dir = tempdir().unwrap();
        let sink = FileLogSink::from_config(
            &FileLogConfig {
                enabled: true,
                dir: Some(dir.path().join("logs")),
                max_bytes: 1024,
                max_files: 2,
            },
            dir.path(),
        )
        .unwrap()
        .unwrap();

        sink.append(&super::LogEntry {
            sequence: 1,
            timestamp_ms: 123,
            level: "info".to_string(),
            message: "hello".to_string(),
            metadata: super::LogMetadata::default(),
        })
        .unwrap();
        sink.flush().unwrap();
        let content = std::fs::read_to_string(sink.path()).unwrap();
        assert!(content.contains("\"timestamp_ms\":123"));
        assert!(content.contains("\"level\":\"info\""));
        assert!(content.contains("\"message\":\"hello\""));
    }

    #[test]
    fn file_log_sink_rotates_and_clears_off_the_caller() {
        let dir = tempdir().unwrap();
        let sink = FileLogSink::from_config(
            &FileLogConfig {
                enabled: true,
                dir: Some(dir.path().join("logs")),
                max_bytes: 1024,
                max_files: 1,
            },
            dir.path(),
        )
        .unwrap()
        .unwrap();
        let line = "x".repeat(900);
        for sequence in 1..=3 {
            sink.append(&super::LogEntry {
                sequence,
                timestamp_ms: sequence,
                level: "info".to_string(),
                message: line.clone(),
                metadata: super::LogMetadata::default(),
            })
            .unwrap();
        }
        sink.flush().unwrap();
        let rotated = std::path::PathBuf::from(format!("{}.1", sink.path().display()));
        assert!(
            rotated.is_file(),
            "expected rotated log {}",
            rotated.display()
        );
        sink.clear().unwrap();
        sink.flush().unwrap();
        assert_eq!(std::fs::read_to_string(sink.path()).unwrap(), "");
    }
}
