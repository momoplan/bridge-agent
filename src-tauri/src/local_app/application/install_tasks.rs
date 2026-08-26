use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalAppInstallTaskPhase {
    Queued,
    Resolving,
    Downloading,
    Verifying,
    Installing,
    Starting,
    Finalizing,
    Succeeded,
    Failed,
}

impl LocalAppInstallTaskPhase {
    fn is_active(self) -> bool {
        !matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalAppInstallTaskOperation {
    Install,
    Upgrade,
    Sync,
}

impl LocalAppInstallTaskOperation {
    pub(crate) fn active_label(self) -> &'static str {
        match self {
            Self::Install => "安装",
            Self::Upgrade => "升级",
            Self::Sync => "同步",
        }
    }

    fn queued_message(self) -> &'static str {
        match self {
            Self::Install => "等待开始安装",
            Self::Upgrade => "等待开始升级",
            Self::Sync => "等待开始同步",
        }
    }

    pub(crate) fn succeeded_message(self) -> &'static str {
        match self {
            Self::Install => "应用已安装，可进入应用完成初始化",
            Self::Upgrade => "应用已升级，可继续使用",
            Self::Sync => "应用已同步到最新来源，可继续使用",
        }
    }

    pub(crate) fn failed_message(self) -> &'static str {
        match self {
            Self::Install => "应用安装失败",
            Self::Upgrade => "应用升级失败",
            Self::Sync => "应用同步失败",
        }
    }

    pub(crate) fn phase_message(self, phase: LocalAppInstallTaskPhase) -> Option<&'static str> {
        match (self, phase) {
            (Self::Install, LocalAppInstallTaskPhase::Resolving) => Some("正在解析安装来源"),
            (Self::Install, LocalAppInstallTaskPhase::Verifying) => {
                Some("正在校验应用清单与平台身份")
            }
            (Self::Install, LocalAppInstallTaskPhase::Installing) => Some("正在安装并注册应用"),
            (Self::Install, LocalAppInstallTaskPhase::Starting) => {
                Some("应用已安装，正在启动并检查运行状态")
            }
            (Self::Upgrade, LocalAppInstallTaskPhase::Resolving) => Some("正在解析升级来源"),
            (Self::Upgrade, LocalAppInstallTaskPhase::Verifying) => {
                Some("正在校验升级包与平台身份")
            }
            (Self::Upgrade, LocalAppInstallTaskPhase::Installing) => {
                Some("正在安装新版本并更新应用注册")
            }
            (Self::Upgrade, LocalAppInstallTaskPhase::Starting) => {
                Some("新版本已安装，正在启动并检查运行状态")
            }
            (Self::Sync, LocalAppInstallTaskPhase::Resolving) => Some("正在解析同步来源"),
            (Self::Sync, LocalAppInstallTaskPhase::Verifying) => Some("正在校验来源包与应用身份"),
            (Self::Sync, LocalAppInstallTaskPhase::Installing) => Some("正在同步并更新应用注册"),
            (Self::Sync, LocalAppInstallTaskPhase::Starting) => {
                Some("应用已同步，正在启动并检查运行状态")
            }
            (_, LocalAppInstallTaskPhase::Finalizing) => Some("正在刷新本地应用能力"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalAppInstallTask {
    pub(crate) task_id: String,
    pub(crate) operation: LocalAppInstallTaskOperation,
    pub(crate) app_id: Option<String>,
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) phase: LocalAppInstallTaskPhase,
    pub(crate) progress_percent: Option<u8>,
    pub(crate) downloaded_bytes: Option<u64>,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) message: String,
    pub(crate) error: Option<String>,
    pub(crate) created_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
}

#[derive(Clone, Default)]
pub(crate) struct LocalAppInstallTaskManager {
    tasks: Arc<RwLock<BTreeMap<String, LocalAppInstallTask>>>,
}

impl LocalAppInstallTaskManager {
    pub(crate) fn create(
        &self,
        operation: LocalAppInstallTaskOperation,
        app_id: Option<String>,
        name: String,
        version: Option<String>,
    ) -> Result<LocalAppInstallTask, String> {
        let mut tasks = self
            .tasks
            .write()
            .map_err(|_| "本地应用安装任务状态锁已损坏".to_string())?;
        if let Some(existing) = tasks
            .values()
            .find(|task| task.phase.is_active() && app_id.is_some() && task.app_id == app_id)
        {
            return Err(format!(
                "应用 {} 已在{}中",
                existing.name,
                existing.operation.active_label()
            ));
        }
        let timestamp = now_ms();
        let task = LocalAppInstallTask {
            task_id: uuid::Uuid::new_v4().to_string(),
            operation,
            app_id,
            name,
            version,
            phase: LocalAppInstallTaskPhase::Queued,
            progress_percent: Some(0),
            downloaded_bytes: None,
            total_bytes: None,
            message: operation.queued_message().to_string(),
            error: None,
            created_at_epoch_ms: timestamp,
            updated_at_epoch_ms: timestamp,
        };
        tasks.insert(task.task_id.clone(), task.clone());
        Ok(task)
    }

    pub(crate) fn update(
        &self,
        task_id: &str,
        update: impl FnOnce(&mut LocalAppInstallTask),
    ) -> Option<LocalAppInstallTask> {
        let mut tasks = self.tasks.write().ok()?;
        let task = tasks.get_mut(task_id)?;
        update(task);
        task.updated_at_epoch_ms = now_ms();
        Some(task.clone())
    }

    pub(crate) fn list(&self) -> Vec<LocalAppInstallTask> {
        self.tasks
            .read()
            .map(|tasks| tasks.values().cloned().collect())
            .unwrap_or_default()
    }
}

pub(crate) fn format_byte_count(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;
    if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
