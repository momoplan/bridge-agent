    #[test]
    fn example_config_contains_only_builtin_services() {
        let config = AgentConfig::example();
        let service_names = config
            .services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(service_names, vec!["computer", "shell"]);
    }
    #[test]
    fn load_config_removes_legacy_disabled_local_java_example() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config
            .services
            .push(legacy_default_local_java_service(false));

        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let loaded = load_config(&path).unwrap();

        assert!(!loaded
            .services
            .iter()
            .any(|service| service.name == "local-java-service"));
        let migrated = fs::read_to_string(&path).unwrap();
        assert!(!migrated.contains("local-java-service"));
    }

    #[test]
    fn load_config_keeps_user_modified_local_java_service() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config
            .services
            .push(legacy_default_local_java_service(true));

        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let loaded = load_config(&path).unwrap();

        assert!(loaded
            .services
            .iter()
            .any(|service| service.name == "local-java-service"));
    }

    fn legacy_default_local_java_service(enabled: bool) -> ServiceConfig {
        ServiceConfig {
            name: "local-java-service".to_string(),
            description: "Example business service backed by a local HTTP endpoint.".to_string(),
            enabled,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "invokeApi".to_string(),
                description: "Forward invocation arguments to a local HTTP service.".to_string(),
                enabled: true,
                input_schema: super::default_object_schema(),
                response_mode: ResponseMode::Cmodel,
                binding: MethodBinding::Http(HttpBinding {
                    url: "http://127.0.0.1:8081/api/invoke".to_string(),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(20),
                }),
            }],
        }
    }

    #[test]
    fn default_shell_exec_allowlist_includes_common_runtimes() {
        let allowlist = default_shell_exec_allow_commands();
        for command in ["node", "npm", "npx", "python3", "python", "pip3", "uv"] {
            assert!(allowlist.contains(&command.to_string()));
        }
    }
