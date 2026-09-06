    #[test]
    fn public_service_registration_builds_http_service_config() {
        let registration: ServiceRegistration = serde_json::from_value(json!({
            "name": "reportTool",
            "description": "AI generated report service.",
            "transport": {
                "type": "http",
                "baseUrl": "http://127.0.0.1:39127/api/",
                "headers": {
                    "x-tool": "report"
                }
            },
            "healthCheck": {
                "type": "http",
                "path": "/health",
                "timeoutSecs": 2,
                "expectStatus": 200
            },
            "startCommand": {
                "type": "shell_command",
                "command": ["report-tool", "start"],
                "timeoutSecs": 10
            },
            "methods": [
                {
                    "name": "generate",
                    "description": "Generate a report.",
                    "path": "/invoke/generate",
                    "timeoutSecs": 60,
                    "input_schema": {
                        "type": "object",
                        "additionalProperties": true
                    }
                }
            ],
            "events": [
                {
                    "name": "finished",
                    "description": "Report generation finished."
                }
            ],
            "replace": true
        }))
        .unwrap();

        let replace = registration.replace;
        let service = registration.into_service_config().unwrap();
        assert!(replace);
        assert_eq!(service.name, "reportTool");
        assert!(serde_json::to_value(&service)
            .unwrap()
            .get("events")
            .is_none());
        match service.health_check.as_ref().unwrap() {
            ServiceHealthCheck::Http {
                url,
                timeout_secs,
                expect_status,
                ..
            } => {
                assert_eq!(url, "http://127.0.0.1:39127/api/health");
                assert_eq!(*timeout_secs, Some(2));
                assert_eq!(*expect_status, Some(200));
            }
        }
        match service.start_command.as_ref().unwrap() {
            ServiceStartCommand::ShellCommand {
                command,
                timeout_secs,
                ..
            } => {
                assert_eq!(
                    command,
                    &vec!["report-tool".to_string(), "start".to_string()]
                );
                assert_eq!(*timeout_secs, Some(10));
            }
        }
        match &service.methods[0].binding {
            MethodBinding::Http(binding) => {
                assert_eq!(binding.url, "http://127.0.0.1:39127/api/invoke/generate");
                assert_eq!(binding.http_method, "POST");
                assert_eq!(binding.timeout_secs, Some(60));
                assert_eq!(binding.headers.get("x-tool").unwrap(), "report");
            }
            other => panic!("unexpected binding: {other:?}"),
        }
    }
