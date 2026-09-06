fn normalized_app_id(app_id: &str) -> Result<String, String> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return Err("本地应用 ID 不能为空".to_string());
    }
    Ok(app_id.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn management_is_available_only_for_the_observed_ready_generation() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();

        assert_eq!(
            manager
                .require_management_ready("connector.test")
                .unwrap_err()
                .code,
            "connector_not_ready"
        );

        manager
            .complete_ready(operation, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let ready = manager.try_management_permit("connector.test").unwrap();
        assert_eq!(ready.lifecycle.lifecycle, ConnectorLifecycleState::Ready);
        assert_eq!(
            ready.lifecycle.desired_generation,
            ready.lifecycle.observed_generation
        );
    }

    #[tokio::test]
    async fn active_operation_serializes_lifecycle_changes() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Upgrade,
                Some("2.0.0".to_string()),
                "upgrading",
            )
            .await
            .unwrap();

        let error = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                None,
                "starting",
            )
            .await
            .err()
            .expect("a second lifecycle operation must be rejected");
        assert!(error.contains("正在执行"));
        manager.fail(operation, "test cleanup").unwrap();
    }

    #[tokio::test]
    async fn health_observation_cannot_overwrite_an_active_transition() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Upgrade,
                Some("2.0.0".to_string()),
                "switching",
            )
            .await
            .unwrap();

        let snapshot = manager
            .observe(
                "connector.test",
                ConnectorLifecycleState::Ready,
                ConnectorHealthState::Healthy,
                Some("1.0.0".to_string()),
                Some(10),
                Some("stale health result".to_string()),
            )
            .unwrap();
        assert_eq!(snapshot.lifecycle, ConnectorLifecycleState::Upgrading);
        assert!(snapshot.operation.is_some());
        manager.fail(operation, "test cleanup").unwrap();
    }

    #[tokio::test]
    async fn upgrade_drains_inflight_management_and_rejects_new_calls() {
        let manager = ConnectorLifecycleManager::default();
        let start = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(start, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let inflight = manager.try_management_permit("connector.test").unwrap();

        let upgrade_manager = manager.clone();
        let upgrade_task = tokio::spawn(async move {
            upgrade_manager
                .begin(
                    "connector.test",
                    ConnectorOperationKind::Upgrade,
                    Some("2.0.0".to_string()),
                    "upgrading",
                )
                .await
        });
        tokio::task::yield_now().await;

        let transitioning = manager
            .list()
            .into_iter()
            .find(|snapshot| snapshot.app_id == "connector.test")
            .unwrap();
        assert_eq!(transitioning.lifecycle, ConnectorLifecycleState::Upgrading);
        assert!(!upgrade_task.is_finished());
        assert!(manager.try_management_permit("connector.test").is_err());

        drop(inflight);
        let upgrade = upgrade_task.await.unwrap().unwrap();
        assert!(manager.try_management_permit("connector.test").is_err());
        manager.fail(upgrade, "test cleanup").unwrap();
    }

    #[tokio::test]
    async fn cancelled_upgrade_wait_restores_the_previous_ready_state() {
        let manager = ConnectorLifecycleManager::default();
        let start = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(start, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let inflight = manager.try_management_permit("connector.test").unwrap();

        let upgrade_manager = manager.clone();
        let upgrade_task = tokio::spawn(async move {
            upgrade_manager
                .begin(
                    "connector.test",
                    ConnectorOperationKind::Upgrade,
                    Some("2.0.0".to_string()),
                    "upgrading",
                )
                .await
        });
        tokio::task::yield_now().await;
        upgrade_task.abort();
        let cancelled = upgrade_task.await;
        assert!(matches!(cancelled, Err(error) if error.is_cancelled()));

        let restored = manager
            .list()
            .into_iter()
            .find(|snapshot| snapshot.app_id == "connector.test")
            .unwrap();
        assert_eq!(restored.lifecycle, ConnectorLifecycleState::Ready);
        assert!(restored.operation.is_none());
        drop(inflight);
        assert!(manager.try_management_permit("connector.test").is_ok());
    }

    #[tokio::test]
    async fn connector_access_gates_are_isolated_by_app_id() {
        let manager = ConnectorLifecycleManager::default();
        let first_start = manager
            .begin(
                "connector.first",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(first_start, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let first_request = manager.try_management_permit("connector.first").unwrap();

        let second_start = manager
            .begin(
                "connector.second",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(second_start, Some("1.0.0".to_string()), Some(84), "ready")
            .unwrap();
        assert!(manager.try_management_permit("connector.second").is_ok());
        drop(first_request);
    }
}
