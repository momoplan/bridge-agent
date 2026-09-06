    #[test]
    fn terminating_conflicting_runtime_refuses_current_process_owner() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let lock_path = runtime_lock_path(&config_path);
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let lock = RuntimeLockDocument {
            pid: std::process::id(),
            agent_id: "dev_1".to_string(),
            config_path: config_path.display().to_string(),
            started_at_ms: 1,
        };
        fs::write(&lock_path, serde_json::to_string_pretty(&lock).unwrap()).unwrap();

        let err =
            terminate_runtime_lock_owner(&lock_path, lock.pid, &lock.agent_id, &lock.config_path)
                .unwrap_err();

        assert!(err.to_string().contains("owned by this 百积木 process"));
        assert!(lock_path.exists());
    }

    #[test]
    fn runtime_process_identity_accepts_desktop_executable_name() {
        let process = RuntimeProcessInfo {
            pid: 13304,
            parent_pid: None,
            name: Some("bridge-agent-desktop.exe".to_string()),
            executable_path: Some(
                r#"C:\Program Files\百积木\bridge-agent-desktop.exe"#.to_string(),
            ),
            command_line: None,
            running: true,
        };

        assert!(process_looks_like_bridge_agent(&process));
    }

    #[test]
    fn runtime_process_identity_accepts_quoted_install_path() {
        assert!(command_line_starts_with_bridge_agent(
            r#""C:\Program Files\百积木\百积木.exe" --config agent-config.json"#
        ));
        assert!(command_line_starts_with_bridge_agent(
            r#""C:\Program Files\百积木\bridge-agent-desktop.exe" --config agent-config.json"#
        ));
    }

    #[test]
    fn runtime_process_identity_rejects_unknown_or_helper_processes() {
        let process = RuntimeProcessInfo {
            pid: 18080,
            parent_pid: None,
            name: Some("my-bridge-agent-helper.exe".to_string()),
            executable_path: None,
            command_line: None,
            running: true,
        };

        assert!(!process_looks_like_bridge_agent(&process));
        assert!(!command_line_starts_with_bridge_agent(
            r#""C:\Program Files\nodejs\node.exe" bridge-agent.js"#
        ));
    }
