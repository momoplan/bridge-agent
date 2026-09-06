    fn assert_generated_agent_id(agent_id: &str) {
        assert!(agent_id.starts_with("dev_"));
        assert_ne!(agent_id, "devbox");
    }

    #[test]
    fn example_config_is_valid() {
        AgentConfig::example().validate().unwrap();
    }

    #[test]
    fn development_and_production_use_distinct_default_config_files() {
        assert_eq!(
            config_file_name_for_build(true),
            "agent-config.development.json"
        );
        assert_eq!(config_file_name_for_build(false), "agent-config.json");
    }

    #[test]
    fn default_device_name_uses_user_and_host() {
        assert_eq!(
            format_default_device_name(Some("alice"), Some("workstation-1")),
            "alice@workstation-1"
        );
        assert_eq!(
            format_default_device_name(Some("alice"), Some("alice")),
            "alice"
        );
        assert_eq!(format_default_device_name(Some("alice"), None), "alice");
        assert_eq!(
            format_default_device_name(None, Some("workstation-1")),
            "workstation-1"
        );
    }

    #[test]
    fn config_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let config = AgentConfig::example();
        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.relay.agent_id, config.relay.agent_id);
        assert_generated_agent_id(&loaded.relay.agent_id);
        assert_eq!(
            loaded.upload.prepare_url(&loaded.relay).as_deref(),
            Some("https://relay.baijimu.com/api/bridge-agent/uploads/prepare")
        );
        assert_eq!(loaded.services.len(), 2);
        assert!(loaded.services.iter().any(|service| service.name == "shell"
            && service.methods.iter().any(|method| method.name == "exec")));
        let shell_methods = loaded
            .services
            .iter()
            .find(|service| service.name == "shell")
            .unwrap()
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>();
        for method in [
            "exec",
            "startExecution",
            "queryExecution",
            "cancelExecution",
        ] {
            assert!(shell_methods.contains(&method));
        }
    }

    #[test]
    fn save_config_keeps_relay_token_out_of_public_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.relay.token = "relay-secret".to_string();

        save_config(&path, &config).unwrap();

        let public_config = fs::read_to_string(&path).unwrap();
        assert!(!public_config.contains("relay-secret"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&public_config).unwrap()["relay"]["token"],
            ""
        );
        assert_eq!(load_config(&path).unwrap().relay.token, "relay-secret");
    }

    #[test]
    fn load_config_migrates_legacy_plaintext_relay_token() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.relay.token = "legacy-secret".to_string();
        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.relay.token, "legacy-secret");
        assert!(!fs::read_to_string(&path).unwrap().contains("legacy-secret"));
        assert!(
            !fs::read_to_string(path.with_file_name("agent-config.json.bak"))
                .unwrap()
                .contains("legacy-secret")
        );
        assert_eq!(load_config(&path).unwrap().relay.token, "legacy-secret");
    }

    #[test]
    fn load_config_rejects_legacy_local_app_installation_identity() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = serde_json::to_value(AgentConfig::example()).unwrap();
        config["local_apps"] = json!([{
            "appId": "com.baijimu.connector.test",
            "installationId": "legacy-installation",
            "name": "Test",
            "version": "1.0.0",
            "events": [{
                "name": "message.received",
                "description": "Message received"
            }]
        }]);
        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let error = load_config(&path).unwrap_err();
        assert!(error.to_string().contains("failed to parse config"));
    }

    #[test]
    fn load_config_removes_legacy_codex_binary_overrides() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = serde_json::to_value(AgentConfig::example()).unwrap();
        config["runtime"]["codex_binary_path"] = json!("/legacy/codex");
        config["services"][0]["start_command"] = json!({
            "type": "shell_command",
            "command": ["example-service"],
            "env": {"CODEX_CONNECTOR_CODEX_BINARY": "/legacy/codex"}
        });
        config["local_apps"] = json!([{
            "appId": "com.baijimu.connector.test",
            "name": "Test",
            "version": "1.0.0",
            "events": [{
                "name": "test.completed",
                "description": "Test completed"
            }],
            "stopCommand": {
                "type": "shell_command",
                "command": ["example-connector", "stop"],
                "env": {"CODEX_CONNECTOR_CODEX_BINARY": "/legacy/codex"}
            }
        }]);
        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let loaded = load_config(&path).unwrap();
        let ServiceStartCommand::ShellCommand { env, .. } =
            loaded.services[0].start_command.as_ref().unwrap();
        assert!(!env.contains_key("CODEX_CONNECTOR_CODEX_BINARY"));
        let ServiceStartCommand::ShellCommand { env, .. } =
            loaded.local_apps[0].stop_command.as_ref().unwrap();
        assert!(!env.contains_key("CODEX_CONNECTOR_CODEX_BINARY"));

        let migrated = fs::read_to_string(&path).unwrap();
        assert!(!migrated.contains("codex_binary_path"));
        assert!(!migrated.contains("CODEX_CONNECTOR_CODEX_BINARY"));
    }

    #[cfg(unix)]
    #[test]
    fn save_config_uses_private_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let config_dir = dir.path().join("bridge-agent");
        let path = config_dir.join("agent-config.json");

        save_config(&path, &AgentConfig::example()).unwrap();

        assert_eq!(
            fs::metadata(&config_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn save_config_preserves_last_config_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.device.name = "first".to_string();
        save_config(&path, &config).unwrap();

        config.device.name = "second".to_string();
        save_config(&path, &config).unwrap();

        let backup_path = path.with_file_name("agent-config.json.bak");
        let backup = fs::read_to_string(backup_path).unwrap();
        assert!(backup.contains("\"name\": \"first\""));
        let current = load_config(&path).unwrap();
        assert_eq!(current.device.name, "second");
    }

    #[test]
    fn load_config_migrates_legacy_default_device_name_when_identity_is_available() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.device.name = "我的百积木".to_string();
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();

        if loaded.device.name != "我的百积木" {
            assert!(!loaded.device.name.trim().is_empty());
        }
    }

    #[test]
    fn load_config_preserves_custom_device_name() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.device.name = "会议室开发机".to_string();
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.device.name, "会议室开发机");
    }

    #[test]
    fn reset_invalid_config_archives_bad_file_and_recreates_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        fs::write(&path, "{").unwrap();

        let recovery = reset_invalid_config(&path).unwrap();

        let archived_path = recovery.archived_path.unwrap();
        assert!(archived_path.exists());
        assert_eq!(fs::read_to_string(archived_path).unwrap(), "{");
        assert_generated_agent_id(&recovery.config.relay.agent_id);
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.services.len(), 2);
    }
