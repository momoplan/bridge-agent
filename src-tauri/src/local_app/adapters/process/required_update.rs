impl ConnectorProcessManager {
    /// Once an update is required, finish admitted starts, stop their processes,
    /// and reject every later start from UI, HTTP, install or automatic startup.
    pub(crate) async fn stop_for_required_update(
        &self,
        config_path: &Path,
        reason: String,
    ) -> Vec<String> {
        let mut admission = self.start_admission.write().await;
        *admission = Some(reason);
        self.stop_all(config_path).await
    }
}

#[cfg(test)]
mod required_update_tests {
    use super::*;

    #[tokio::test]
    async fn required_update_blocks_later_connector_starts_before_installation_lookup() {
        let manager = ConnectorProcessManager::default();
        let dir = tempfile::tempdir().unwrap();
        assert!(manager.stop_for_required_update(dir.path(), "需要更新".into()).await.is_empty());
        let error = manager.start("uninstalled-app", dir.path(), Default::default()).await.unwrap_err();
        assert!(error.contains("必须先升级"), "{error}");
    }

    #[tokio::test]
    async fn required_update_shutdown_waits_for_an_admitted_start_to_finish() {
        let manager = ConnectorProcessManager::default();
        let dir = tempfile::tempdir().unwrap();
        let in_flight_start = manager.start_admission.read().await;
        let (entered_tx, entered_rx) = oneshot::channel();
        let stopping = manager.clone();
        let stop = tokio::spawn(async move {
            entered_tx.send(()).unwrap();
            stopping.stop_for_required_update(dir.path(), "需要更新".into()).await
        });
        entered_rx.await.unwrap();
        assert!(!stop.is_finished(), "shutdown must wait until process creation finishes");
        drop(in_flight_start);
        assert!(stop.await.unwrap().is_empty());
        assert!(manager.start("uninstalled-app", Path::new("unused"), Default::default()).await.unwrap_err().contains("必须先升级"));
    }
}
