    #[tokio::test]
    async fn registry_exposes_enabled_service_definitions() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let definitions = registry.definitions();
        assert_eq!(definitions.len(), 2);
        assert!(definitions.iter().any(|service| service.name == "computer"
            && service
                .methods
                .iter()
                .any(|method| method.name == "screenshot")));
        assert!(definitions.iter().any(|service| service.name == "shell"
            && service.methods.iter().any(|method| method.name == "exec")));
        assert!(definitions.iter().any(|service| service.name == "shell"
            && service
                .methods
                .iter()
                .any(|method| method.name == "queryExecution")));
    }

    #[tokio::test]
    async fn shell_execution_can_be_started_and_polled() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let start = registry
            .invoke(
                "req-start".to_string(),
                "shell",
                "startExecution",
                json!({"command": ["echo", "bridge-agent-ok"]}),
                Some(5),
            )
            .await;
        assert!(start.success);
        let execution_id = start.data.unwrap()["executionId"]
            .as_str()
            .unwrap()
            .to_string();

        let mut status = String::new();
        let mut stdout = String::new();
        for _ in 0..20 {
            let result = registry
                .invoke(
                    "req-get".to_string(),
                    "shell",
                    "queryExecution",
                    json!({"executionId": execution_id}),
                    None,
                )
                .await;
            assert!(result.success);
            let data = result.data.unwrap();
            status = data["status"].as_str().unwrap().to_string();
            stdout = data["stdout"].as_str().unwrap_or_default().to_string();
            if status != "RUNNING" {
                assert_eq!(data["exitCode"].as_i64(), Some(0));
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        assert_eq!(status, "SUCCEEDED");
        assert!(stdout.contains("bridge-agent-ok"));
    }

    #[tokio::test]
    async fn shell_execution_writes_stdin_to_process() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let start = registry
            .invoke(
                "req-start-stdin".to_string(),
                "shell",
                "startExecution",
                json!({
                    "command": ["sh", "-lc", "read value; printf 'stdin:%s' \"$value\""],
                    "stdin": "bridge-agent-stdin\n"
                }),
                Some(5),
            )
            .await;
        assert!(start.success);
        let execution_id = start.data.unwrap()["executionId"]
            .as_str()
            .unwrap()
            .to_string();

        let mut status = String::new();
        let mut stdout = String::new();
        for _ in 0..200 {
            let result = registry
                .invoke(
                    "req-get-stdin".to_string(),
                    "shell",
                    "queryExecution",
                    json!({"executionId": execution_id}),
                    None,
                )
                .await;
            assert!(result.success);
            let data = result.data.unwrap();
            status = data["status"].as_str().unwrap().to_string();
            stdout = data["stdout"].as_str().unwrap_or_default().to_string();
            if status != "RUNNING" {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        assert_eq!(status, "SUCCEEDED");
        assert_eq!(stdout, "stdin:bridge-agent-stdin");
    }

    #[tokio::test]
    async fn shell_query_execution_returns_running_output_tail() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let start = registry
            .invoke(
                "req-start-live-output".to_string(),
                "shell",
                "startExecution",
                json!({
                    "command": [
                        "sh",
                        "-lc",
                        "printf '%s' bridge-agent-live-out; printf '%s' bridge-agent-live-err >&2; sleep 1; printf '%s' -done"
                    ]
                }),
                Some(5),
            )
            .await;
        assert!(start.success);
        let execution_id = start.data.unwrap()["executionId"]
            .as_str()
            .unwrap()
            .to_string();

        let mut saw_running_stdout = false;
        let mut saw_running_stderr = false;
        let mut status = String::new();
        let mut final_stdout = String::new();
        let mut final_stderr = String::new();
        for _ in 0..80 {
            let result = registry
                .invoke(
                    "req-get-live-output".to_string(),
                    "shell",
                    "queryExecution",
                    json!({"executionId": execution_id}),
                    None,
                )
                .await;
            assert!(result.success);
            let data = result.data.unwrap();
            status = data["status"].as_str().unwrap().to_string();
            let stdout = data["stdout"].as_str().unwrap_or_default().to_string();
            let stderr = data["stderr"].as_str().unwrap_or_default().to_string();

            if status == "RUNNING" {
                saw_running_stdout |= stdout.contains("bridge-agent-live-out");
                saw_running_stderr |= stderr.contains("bridge-agent-live-err");
            } else {
                final_stdout = stdout;
                final_stderr = stderr;
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }

        assert_eq!(status, "SUCCEEDED");
        assert!(
            saw_running_stdout,
            "expected queryExecution to expose stdout while running"
        );
        assert!(
            saw_running_stderr,
            "expected queryExecution to expose stderr while running"
        );
        assert!(final_stdout.contains("bridge-agent-live-out-done"));
        assert!(final_stderr.contains("bridge-agent-live-err"));
    }

    #[tokio::test]
    async fn shell_start_execution_does_not_use_invoke_timeout_as_process_timeout() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let start = registry
            .invoke(
                "req-start-no-process-timeout".to_string(),
                "shell",
                "startExecution",
                json!({"command": ["sh", "-lc", "sleep 2; echo bridge-agent-late"]}),
                Some(1),
            )
            .await;
        assert!(start.success);
        let start_data = start.data.unwrap();
        assert_eq!(start_data["timeoutSecs"], Value::Null);
        let execution_id = start_data["executionId"].as_str().unwrap().to_string();

        let mut status = String::new();
        let mut stdout = String::new();
        for _ in 0..80 {
            let result = registry
                .invoke(
                    "req-start-no-process-timeout-poll".to_string(),
                    "shell",
                    "queryExecution",
                    json!({"executionId": execution_id}),
                    None,
                )
                .await;
            assert!(result.success);
            let data = result.data.unwrap();
            status = data["status"].as_str().unwrap().to_string();
            stdout = data["stdout"].as_str().unwrap_or_default().to_string();
            if status != "RUNNING" {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(status, "SUCCEEDED");
        assert!(stdout.contains("bridge-agent-late"));
    }

    #[tokio::test]
    async fn shell_start_execution_honors_explicit_argument_timeout() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let start = registry
            .invoke(
                "req-start-explicit-process-timeout".to_string(),
                "shell",
                "startExecution",
                json!({
                    "command": ["sh", "-lc", "sleep 2; echo bridge-agent-too-late"],
                    "timeoutSeconds": 1
                }),
                Some(5),
            )
            .await;
        assert!(start.success);
        let start_data = start.data.unwrap();
        assert_eq!(start_data["timeoutSecs"].as_u64(), Some(1));
        let execution_id = start_data["executionId"].as_str().unwrap().to_string();

        let mut status = String::new();
        for _ in 0..80 {
            let result = registry
                .invoke(
                    "req-start-explicit-process-timeout-poll".to_string(),
                    "shell",
                    "queryExecution",
                    json!({"executionId": execution_id}),
                    None,
                )
                .await;
            assert!(result.success);
            let data = result.data.unwrap();
            status = data["status"].as_str().unwrap().to_string();
            if status != "RUNNING" {
                assert_eq!(data["timedOut"].as_bool(), Some(true));
                assert_eq!(data["error"]["code"].as_str(), Some("TIMEOUT"));
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }

        assert_eq!(status, "TIMED_OUT");
    }

    #[tokio::test]
    async fn shell_exec_returns_execution_metadata_for_quick_commands() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-shell-quick".to_string(),
                "shell",
                "exec",
                json!({"command": ["echo", "bridge-agent-ok"]}),
                Some(5),
            )
            .await;

        assert!(result.success);
        let mut data = result.data.unwrap();
        if data["status"].as_str() == Some("RUNNING") {
            let execution_id = data["executionId"].as_str().unwrap().to_string();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                let polled = registry
                    .invoke(
                        "req-shell-quick-poll".to_string(),
                        "shell",
                        "queryExecution",
                        json!({"executionId": execution_id}),
                        None,
                    )
                    .await;
                assert!(polled.success);
                data = polled.data.unwrap();
                if data["status"].as_str() != Some("RUNNING") {
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
        }
        assert!(data["executionId"].as_str().is_some());
        assert_eq!(data["status"].as_str(), Some("SUCCEEDED"));
        assert_eq!(data["exitCode"].as_i64(), Some(0));
        assert!(data["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("bridge-agent-ok"));
    }

    #[tokio::test]
    async fn shell_exec_returns_running_execution_for_long_commands() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-shell-running".to_string(),
                "shell",
                "exec",
                json!({"command": ["sh", "-lc", "sleep 1; echo bridge-agent-late"]}),
                // This test verifies the async hand-off after the 100 ms test-only
                // synchronous wait. Keep the process timeout well outside CI
                // scheduler and Git-for-Windows shell startup variance; explicit
                // process-timeout behavior is covered by the dedicated test above.
                Some(30),
            )
            .await;

        assert!(result.success);
        let data = result.data.unwrap();
        let execution_id = data["executionId"].as_str().unwrap().to_string();
        assert_eq!(data["status"].as_str(), Some("RUNNING"));
        assert_eq!(data["recommendedAction"].as_str(), Some("queryExecution"));
        assert_eq!(data["recommendedService"].as_str(), Some("shell"));
        assert_eq!(data["recommendedMethod"].as_str(), Some("queryExecution"));
        assert_eq!(data["exitCode"], Value::Null);
        assert_eq!(data["timeoutSecs"].as_u64(), Some(30));

        let deadline = Instant::now() + Duration::from_secs(15);
        let (status, stdout) = loop {
            let polled = registry
                .invoke(
                    "req-shell-running-poll".to_string(),
                    "shell",
                    "queryExecution",
                    json!({"executionId": execution_id}),
                    None,
                )
                .await;
            assert!(polled.success);
            let data = polled.data.unwrap();
            let status = data["status"].as_str().unwrap().to_string();
            let stdout = data["stdout"].as_str().unwrap_or_default().to_string();
            if status != "RUNNING" || Instant::now() >= deadline {
                break (status, stdout);
            }
            sleep(Duration::from_millis(50)).await;
        };

        assert_eq!(status, "SUCCEEDED");
        assert!(stdout.contains("bridge-agent-late"));
    }
