    #[test]
    fn load_legacy_config_without_platform() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        fs::write(
            &path,
            r#"{
  "upload": {
    "prepare_url": null,
    "inline_limit_bytes": 8388608,
    "timeout_secs": 60
  },
  "relay": {
    "url": "ws://127.0.0.1:8080/ws/agent",
    "agent_id": "devbox",
    "token": "",
    "reconnect_secs": 3
  },
  "device": {
    "name": "我的百积木",
    "description": "Installed on the user's local machine.",
    "tags": ["desktop", "local"]
  },
  "runtime": {
    "default_timeout_secs": 30,
    "max_timeout_secs": 120,
    "log_limit": 500
  },
  "services": []
}"#,
        )
        .unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.platform.base_url, "https://api.baijimu.com/lowcode3");
        assert_eq!(loaded.platform.workspace_id, None);
        assert_eq!(loaded.relay.url, "wss://relay.baijimu.com/ws/agent");
        assert_eq!(loaded.upload.inline_limit_bytes, 256 * 1024);
        assert_eq!(loaded.relay.agent_id, "devbox");
        assert!(loaded.runtime.service_registration_enabled);
        assert!(loaded
            .runtime
            .service_registration_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty()));
        let migrated = fs::read_to_string(&path).unwrap();
        assert!(migrated.contains("service_registration_token"));
        assert_eq!(loaded.services[0].name, "computer");
        assert!(loaded.services[0]
            .methods
            .iter()
            .any(|method| method.name == "screenshot"));
        assert!(loaded
            .services
            .iter()
            .any(|service| service.name == "shell"));
    }

    #[test]
    fn normalizes_common_baijimu_platform_entrypoints() {
        for legacy_url in [
            "https://baijimu.com",
            "https://www.baijimu.com/",
            "https://baijimu.com/lowcode",
            "https://baijimu.com/manager",
            "https://www.baijimu.com/lowcode3/",
            "https://api.baijimu.com",
            "https://api.baijimu.com/lowcode3/",
        ] {
            let dir = tempdir().unwrap();
            let path = dir.path().join("agent-config.json");
            let mut config = AgentConfig::example();
            config.platform.base_url = legacy_url.to_string();
            save_config(&path, &config).unwrap();

            let loaded = load_config(&path).unwrap();
            assert_eq!(
                loaded.platform.base_url, "https://api.baijimu.com/lowcode3",
                "legacy url {legacy_url} should normalize to the production API prefix"
            );
        }
    }

    #[test]
    fn leaves_custom_platform_base_url_unchanged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.base_url = "https://dev.baijimu.test/lowcode3".to_string();
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(
            loaded.platform.base_url,
            "https://dev.baijimu.test/lowcode3"
        );
    }

    #[test]
    fn non_loopback_event_server_disables_service_registration_migration() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.runtime.event_server_bind = "0.0.0.0:18081".to_string();
        config.runtime.service_registration_token = None;
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();
        assert!(!loaded.runtime.service_registration_enabled);
    }

    #[test]
    fn load_legacy_computer_service_adds_missing_default_methods() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        fs::write(
            &path,
            r#"{
  "platform": {
    "base_url": "https://baijimu.com/lowcode3",
    "workspace_id": null
  },
  "upload": {
    "prepare_url": null,
    "inline_limit_bytes": 8388608,
    "timeout_secs": 60
  },
  "relay": {
    "url": "wss://relay.baijimu.com/ws/agent",
    "agent_id": "devbox",
    "token": "",
    "reconnect_secs": 3
  },
  "device": {
    "name": "我的百积木",
    "description": "Installed on the user's local machine.",
    "tags": ["desktop", "local"]
  },
  "runtime": {
    "default_timeout_secs": 30,
    "max_timeout_secs": 120,
    "log_limit": 500
  },
  "services": [
    {
      "name": "computer",
      "description": "legacy",
      "enabled": true,
      "methods": [
        {
          "name": "exec",
          "description": "legacy shell",
          "enabled": true,
          "input_schema": {
            "type": "object"
          },
          "binding": {
            "type": "shell_command",
            "root_dir": ".",
            "allow_commands": ["echo"]
          }
        }
      ]
    }
  ]
}"#,
        )
        .unwrap();

        let loaded = load_config(&path).unwrap();
        let computer = loaded
            .services
            .iter()
            .find(|service| service.name == "computer")
            .unwrap();
        assert!(computer.methods.iter().any(|method| method.name == "exec"));
        assert!(computer
            .methods
            .iter()
            .any(|method| method.name == "screenshot"));
        assert!(computer.methods.iter().any(|method| method.name == "click"));
    }
