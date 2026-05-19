use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    max_bytes: u64,
    max_files: usize,
}

#[derive(Serialize)]
struct FileLogRecord<'a> {
    timestamp_ms: u64,
    level: &'a str,
    message: &'a str,
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

        Ok(Some(Self {
            path: dir.join("bridge-agent.log"),
            max_bytes: config.max_bytes.max(1024),
            max_files: config.max_files.max(1),
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, timestamp_ms: u64, level: &str, message: &str) -> Result<()> {
        self.rotate_if_needed()?;
        let record = FileLogRecord {
            timestamp_ms,
            level,
            message,
        };
        let line = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open log file {}", self.path.display()))?;
        writeln!(file, "{line}")
            .with_context(|| format!("failed to write log file {}", self.path.display()))?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
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
        Ok(())
    }

    fn rotate_if_needed(&self) -> Result<()> {
        let Ok(metadata) = fs::metadata(&self.path) else {
            return Ok(());
        };
        if metadata.len() < self.max_bytes {
            return Ok(());
        }

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
        let _ = fs::rename(&self.path, first);
        Ok(())
    }
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

        sink.append(123, "info", "hello").unwrap();
        let content = std::fs::read_to_string(sink.path()).unwrap();
        assert!(content.contains("\"timestamp_ms\":123"));
        assert!(content.contains("\"level\":\"info\""));
        assert!(content.contains("\"message\":\"hello\""));
    }
}
