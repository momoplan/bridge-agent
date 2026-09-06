    #[cfg(unix)]
    #[test]
    fn lifecycle_command_restores_host_path_for_nested_system_tools() {
        let command = ServiceStartCommand::ShellCommand {
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "command -v env".to_string(),
            ],
            cwd: None,
            env: BTreeMap::from([("PATH".to_string(), "/connector-only".to_string())]),
            timeout_secs: Some(5),
        };

        let result = run_start_command(
            "com.baijimu.connector.path-test",
            &command,
            &BTreeMap::new(),
        )
        .unwrap();

        assert_eq!(result.exit_code, Some(0), "{}", result.stderr);
        assert!(result.stdout.contains("/env"), "{}", result.stdout);
    }

    #[test]
    fn connector_path_rename_retries_windows_handle_release_errors() {
        assert!(connector_path_error_is_retryable(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "locked",
        )));
        assert!(connector_path_error_is_retryable(
            &std::io::Error::from_raw_os_error(32)
        ));
        assert!(!connector_path_error_is_retryable(&std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing",
        )));
    }

    #[test]
    fn connector_path_rename_preserves_the_operating_system_error_chain() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("missing");
        let destination = dir.path().join("destination");
        let error =
            rename_connector_path_with_retry(&source, &destination, 1, Duration::from_millis(0))
                .unwrap_err();

        assert!(error
            .to_string()
            .contains("after waiting for process handles to close"));
        assert!(error.chain().count() >= 2);
    }

    #[cfg(windows)]
    fn failing_test_command() -> ServiceStartCommand {
        ServiceStartCommand::ShellCommand {
            command: vec![
                "cmd.exe".to_string(),
                "/C".to_string(),
                "exit 7".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
            timeout_secs: Some(5),
        }
    }

    #[cfg(not(windows))]
    fn failing_test_command() -> ServiceStartCommand {
        ServiceStartCommand::ShellCommand {
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "exit 7".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
            timeout_secs: Some(5),
        }
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_command_does_not_wait_for_descendant_owned_stdio() {
        let command = ServiceStartCommand::ShellCommand {
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "sleep 3 & printf started".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
            timeout_secs: Some(2),
        };
        let started_at = Instant::now();

        let result = run_start_command_with_user_path(
            "com.baijimu.connector.pipe-regression",
            &command,
            &BTreeMap::new(),
            None,
        )
        .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "started");
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "lifecycle output collection waited for a descendant-owned handle"
        );
    }

    #[cfg(windows)]
    #[test]
    fn lifecycle_command_does_not_wait_for_descendant_owned_stdio() {
        let command = ServiceStartCommand::ShellCommand {
            command: vec![
                "powershell.exe".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                "Start-Process -FilePath ping.exe -ArgumentList @('127.0.0.1','-n','12') -WindowStyle Hidden | Out-Null; Write-Output started".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
            timeout_secs: Some(8),
        };
        let started_at = Instant::now();

        let result = run_start_command(
            "com.baijimu.connector.pipe-regression",
            &command,
            &BTreeMap::new(),
        )
        .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("started"));
        assert!(
            started_at.elapsed() < Duration::from_secs(8),
            "lifecycle output collection waited for a descendant-owned handle"
        );
    }

    #[test]
    fn lifecycle_output_capture_is_bounded() {
        use std::io::Write as _;

        let mut capture = tempfile::NamedTempFile::new().unwrap();
        capture
            .write_all(&vec![b'x'; LIFECYCLE_OUTPUT_MAX_BYTES as usize + 32])
            .unwrap();

        let output =
            read_lifecycle_capture("com.baijimu.connector.output-limit", "stdout", &capture)
                .unwrap();

        assert!(output.ends_with(b"\n[output truncated by Bridge Agent]"));
        assert_eq!(
            output.len(),
            LIFECYCLE_OUTPUT_MAX_BYTES as usize + b"\n[output truncated by Bridge Agent]".len()
        );
    }

    #[test]
    fn replacement_refuses_running_capable_connector_without_stop_command() {
        let dir = tempdir().unwrap();
        let connectors_dir = dir.path().join("connectors");
        let _env = connector_test_env(connectors_dir.clone());
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        let manifest_path = source.join(CONNECTOR_MANIFEST_FILE);
        let manifest = |version: &str| {
            test_manifest_with_command(
                "com.baijimu.connector.lifecycle-test",
                "Lifecycle Test Connector",
                version,
                "connector-test",
                &["start"],
            )
        };
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest("1.0.0")).unwrap(),
        )
        .unwrap();
        install_connector_from_path(&source, &config_path, false).unwrap();

        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest("1.0.1")).unwrap(),
        )
        .unwrap();
        let error = install_connector_from_path(&source, &config_path, true).unwrap_err();

        assert!(error.to_string().contains("no stop command"));
        assert_eq!(
            show_connector("com.baijimu.connector.lifecycle-test")
                .unwrap()
                .manifest
                .version,
            "1.0.0"
        );
        let installed_manifest: ConnectorManifest = serde_json::from_str(
            &fs::read_to_string(
                connectors_dir
                    .join("com.baijimu.connector.lifecycle-test")
                    .join("package")
                    .join(CONNECTOR_MANIFEST_FILE),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(installed_manifest.version, "1.0.0");
    }

    #[test]
    fn replacement_rollback_restores_previous_package() {
        let dir = tempdir().unwrap();
        let package_path = dir.path().join("package");
        let replaced_path = dir.path().join("package.replaced");
        fs::create_dir_all(&package_path).unwrap();
        fs::write(package_path.join("state"), "old").unwrap();
        fs::rename(&package_path, &replaced_path).unwrap();
        fs::create_dir_all(&package_path).unwrap();
        fs::write(package_path.join("state"), "incomplete-new").unwrap();

        restore_replaced_connector_package(&package_path, Some(&replaced_path)).unwrap();

        assert_eq!(
            fs::read_to_string(package_path.join("state")).unwrap(),
            "old"
        );
        assert!(!replaced_path.exists());
    }

    #[test]
    fn reinstall_preserves_install_time_and_updates_sync_source() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest(
                "com.baijimu.connector.resync",
                "Resync Connector",
                "0.1.0",
            ))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path_with_source_reference(
            &source,
            &config_path,
            false,
            Some("https://example.test/connector.git#main"),
        )
        .unwrap();
        let first = show_connector("com.baijimu.connector.resync").unwrap();

        install_connector_from_path_with_source_reference(
            &source,
            &config_path,
            true,
            Some("https://example.test/connector.git#next"),
        )
        .unwrap();
        let second = show_connector("com.baijimu.connector.resync").unwrap();
        assert_eq!(second.installed_at_epoch_ms, first.installed_at_epoch_ms);
        assert_eq!(
            second.source_reference.as_deref(),
            Some("https://example.test/connector.git#next")
        );
        assert!(second.last_synced_at_epoch_ms >= first.last_synced_at_epoch_ms);

        let summary = list_connectors()
            .unwrap()
            .into_iter()
            .find(|connector| connector.app_id == "com.baijimu.connector.resync")
            .unwrap();
        assert_eq!(summary.source_reference, second.source_reference);
        assert_eq!(
            summary.last_synced_at_epoch_ms,
            second.last_synced_at_epoch_ms
        );
    }
