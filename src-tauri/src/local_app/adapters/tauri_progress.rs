use super::super::application::install_tasks::{
    format_byte_count, LocalAppInstallTask, LocalAppInstallTaskManager, LocalAppInstallTaskPhase,
};

#[derive(Clone)]
pub(crate) struct LocalAppInstallProgressReporter {
    pub(crate) manager: LocalAppInstallTaskManager,
    pub(crate) task_id: String,
    pub(crate) on_event: Option<tauri::ipc::Channel<LocalAppInstallTask>>,
}

impl LocalAppInstallProgressReporter {
    pub(crate) fn send(&self, task: LocalAppInstallTask) {
        let Some(on_event) = self.on_event.as_ref() else {
            return;
        };
        if let Err(err) = on_event.send(task) {
            log::warn!(
                "failed to send local app task progress: task_id={} error={err}",
                self.task_id
            );
        }
    }

    pub(crate) fn update(&self, update: impl FnOnce(&mut LocalAppInstallTask)) {
        if let Some(task) = self.manager.update(&self.task_id, update) {
            self.send(task);
        }
    }

    pub(crate) fn report(
        &self,
        phase: LocalAppInstallTaskPhase,
        progress_percent: Option<u8>,
        message: impl Into<String>,
    ) {
        let fallback_message = message.into();
        self.update(|task| {
            task.phase = phase;
            task.progress_percent = progress_percent;
            task.message = task
                .operation
                .phase_message(phase)
                .unwrap_or(fallback_message.as_str())
                .to_string();
            task.error = None;
            task.downloaded_bytes = None;
            task.total_bytes = None;
        });
    }

    pub(crate) fn download(&self, downloaded_bytes: u64, total_bytes: Option<u64>) {
        self.update(|task| {
            task.phase = LocalAppInstallTaskPhase::Downloading;
            task.downloaded_bytes = Some(downloaded_bytes);
            task.total_bytes = total_bytes;
            task.progress_percent = total_bytes.filter(|total| *total > 0).map(|total| {
                let download_percent = downloaded_bytes.saturating_mul(100) / total;
                (10 + download_percent.saturating_mul(45) / 100).min(55) as u8
            });
            task.message = match total_bytes {
                Some(total) => format!(
                    "正在下载应用包（{} / {}）",
                    format_byte_count(downloaded_bytes),
                    format_byte_count(total)
                ),
                None => format!("正在下载应用包（{}）", format_byte_count(downloaded_bytes)),
            };
        });
    }

    pub(crate) fn identity(&self, app_id: &str, name: &str, version: &str) {
        self.update(|task| {
            task.app_id = Some(app_id.to_string());
            task.name = name.to_string();
            task.version = Some(version.to_string());
        });
    }
}
