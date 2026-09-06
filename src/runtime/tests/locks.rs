    #[test]
    fn runtime_lock_rejects_second_owner_until_released() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");

        let first = RuntimeInstanceLock::acquire(&config_path, "dev_1").unwrap();
        let second = RuntimeInstanceLock::acquire(&config_path, "dev_1");
        assert!(second.is_err());

        drop(first);
        RuntimeInstanceLock::acquire(&config_path, "dev_1").unwrap();
    }

    #[test]
    fn runtime_lock_cannot_be_bypassed_by_agent_identity_change() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");

        let first = RuntimeInstanceLock::acquire(&config_path, "dev_old_identity").unwrap();
        let second = RuntimeInstanceLock::acquire(&config_path, "dev_new_identity");
        assert!(second.is_err());
        drop(first);
    }

    #[test]
    fn runtime_lock_detects_active_legacy_agent_scoped_lock() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let lock_dir = dir.path().join(super::RUNTIME_LOCK_DIR);
        fs::create_dir_all(&lock_dir).unwrap();
        let legacy_lock_path = lock_dir.join("agent-config.json-dev_old_identity-legacy.lock");
        let legacy = RuntimeLockDocument {
            pid: std::process::id(),
            agent_id: "dev_old_identity".to_string(),
            config_path: config_path.display().to_string(),
            started_at_ms: 1,
        };
        fs::write(
            &legacy_lock_path,
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let result = RuntimeInstanceLock::acquire(&config_path, "dev_new_identity");

        assert!(result.is_err());
        assert!(legacy_lock_path.exists());
    }

    #[test]
    fn runtime_lock_removes_stale_owner() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let lock_path = runtime_lock_path(&config_path);
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let stale = RuntimeLockDocument {
            pid: u32::MAX,
            agent_id: "dev_1".to_string(),
            config_path: config_path.display().to_string(),
            started_at_ms: 1,
        };
        fs::write(&lock_path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

        let lock = RuntimeInstanceLock::acquire(&config_path, "dev_1").unwrap();
        let active = read_runtime_lock(&lock.path).unwrap();
        assert_eq!(active.pid, std::process::id());
    }

    #[cfg(unix)]
    #[test]
    fn unix_process_probe_rejects_pid_outside_signed_pid_range() {
        assert!(!super::process_is_running(u32::MAX));
        assert!(!super::process_is_running(i32::MAX as u32 + 1));
    }

    #[test]
    fn runtime_lock_owner_rejects_pid_reused_by_non_bridge_process() {
        let process = RuntimeProcessInfo {
            pid: 727,
            parent_pid: Some(1),
            name: Some("ViewBridgeAuxiliary".to_string()),
            executable_path: Some(
                "/System/Library/PrivateFrameworks/ViewBridge.framework/Versions/A/XPCServices/ViewBridgeAuxiliary.xpc/Contents/MacOS/ViewBridgeAuxiliary".to_string(),
            ),
            command_line: Some(
                "/System/Library/PrivateFrameworks/ViewBridge.framework/Versions/A/XPCServices/ViewBridgeAuxiliary.xpc/Contents/MacOS/ViewBridgeAuxiliary".to_string(),
            ),
            running: true,
        };

        assert!(!runtime_lock_owner_is_active(&process));
    }

    #[test]
    fn runtime_lock_owner_keeps_unidentifiable_running_process() {
        let process = RuntimeProcessInfo {
            pid: 42,
            parent_pid: None,
            name: None,
            executable_path: None,
            command_line: None,
            running: true,
        };

        assert!(runtime_lock_owner_is_active(&process));
    }

    #[test]
    fn runtime_start_active_statuses_are_idempotent_start_targets() {
        assert!(runtime_start_is_active(RuntimeStatus::Starting));
        assert!(runtime_start_is_active(RuntimeStatus::Connecting));
        assert!(runtime_start_is_active(RuntimeStatus::Backoff));
        assert!(runtime_start_is_active(
            RuntimeStatus::AuthorizationRequired
        ));
        assert!(!runtime_start_is_active(RuntimeStatus::Online));
        assert!(!runtime_start_is_active(RuntimeStatus::Stopped));
        assert!(!runtime_start_is_active(RuntimeStatus::Stopping));
    }
