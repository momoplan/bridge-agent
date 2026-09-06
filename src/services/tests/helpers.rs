    fn http_test_service(url: &str) -> ServiceConfig {
        http_test_service_with_mode(url, ResponseMode::Cmodel)
    }

    fn http_test_service_with_mode(url: &str, response_mode: ResponseMode) -> ServiceConfig {
        ServiceConfig {
            name: "localTool".to_string(),
            description: "Local HTTP tool.".to_string(),
            enabled: true,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "fetch".to_string(),
                description: "Fetch data.".to_string(),
                enabled: true,
                input_schema: json!({"type": "object"}),
                response_mode,
                binding: MethodBinding::Http(HttpBinding {
                    url: url.to_string(),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(5),
                }),
            }],
        }
    }
