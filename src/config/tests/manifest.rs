    #[test]
    fn upload_prepare_url_prefers_explicit_value() {
        let mut config = AgentConfig::example();
        config.upload.prepare_url = Some("https://uploads.example.com/prepare".to_string());
        assert_eq!(
            config.upload.prepare_url(&config.relay).as_deref(),
            Some("https://uploads.example.com/prepare")
        );
    }

    #[test]
    fn manifest_preview_contains_enabled_service_only() {
        let payload = manifest_preview_json(&AgentConfig::example()).unwrap();
        assert!(payload.contains("\"computer\""));
        assert!(payload.contains("\"shell\""));
        assert!(!payload.contains("\"local-java-service\""));
    }

    #[test]
    fn browser_auth_manifest_omits_schemas() {
        let mut config = AgentConfig::example();
        config.services.push(ServiceConfig {
            name: "eventful".to_string(),
            description: "Eventful service".to_string(),
            enabled: true,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "doThing".to_string(),
                description: "Does a thing".to_string(),
                enabled: true,
                input_schema: json!({
                    "type": "object",
                    "required": ["large"],
                    "properties": {
                        "large": {
                            "type": "string",
                            "description": "schema-only text"
                        }
                    }
                }),
                response_mode: ResponseMode::Cmodel,
                binding: MethodBinding::Http(HttpBinding {
                    url: "http://127.0.0.1:18000/do-thing".to_string(),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(20),
                }),
            }],
        });

        let payload = browser_auth_manifest_json(&config).unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let service = value["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|service| service["name"] == "eventful")
            .unwrap();

        assert_eq!(service["methods"][0]["name"], "doThing");
        assert!(service["methods"][0].get("input_schema").is_none());
        assert!(!payload.contains("schema-only text"));
    }

    #[test]
    fn shell_manifest_exposes_single_argv_command_schema() {
        let manifest = AgentConfig::example().manifest_preview();
        let shell_method = manifest
            .services
            .iter()
            .find(|service| service.name == "shell")
            .and_then(|service| service.methods.iter().find(|method| method.name == "exec"))
            .unwrap();
        let command_schema = &shell_method.input_schema["properties"]["command"];
        assert_eq!(command_schema["type"], "array");
        assert_eq!(command_schema["items"]["type"], "string");
        assert!(command_schema.get("anyOf").is_none());

        let query_execution_method = manifest
            .services
            .iter()
            .find(|service| service.name == "shell")
            .and_then(|service| {
                service
                    .methods
                    .iter()
                    .find(|method| method.name == "queryExecution")
            })
            .unwrap();
        assert_eq!(
            query_execution_method.input_schema["properties"]["executionId"]["type"],
            "string"
        );
    }
