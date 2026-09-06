    #[test]
    fn install_connector_resolves_package_bin_start_command() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "test-connector",
                "bin": {
                    "test-connector": "./bin/start.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("bin").join("start.js"), "console.log('ok');\n").unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest_with_command(
                "com.baijimu.connector.bin",
                "Bin Connector",
                "0.1.0",
                "test-connector",
                &["--port", "18082"],
            ))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        let package_path = PathBuf::from(result.package_path);
        let config = load_config(&config_path).unwrap();
        let service = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.bin")
            .unwrap();
        let ServiceStartCommand::ShellCommand {
            command, cwd, env, ..
        } = service.start_command.as_ref().unwrap();
        assert_eq!(command[0], "node");
        assert_eq!(
            command[1],
            package_path.join("./bin/start.js").display().to_string()
        );
        assert_eq!(command[2], "--port");
        assert_eq!(cwd.as_deref(), Some(package_path.to_str().unwrap()));
        assert_eq!(
            env.get(LOCAL_APP_DATA_DIR_ENV).map(String::as_str),
            Some(
                connector_data_dir("com.baijimu.connector.bin")
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );
        assert_eq!(
            env.get(LOCAL_APP_ID_ENV).map(String::as_str),
            Some("com.baijimu.connector.bin")
        );
        assert!(!env.contains_key("PATH"));

        prepare_installed_connector_runtime(&config_path, "com.baijimu.connector.bin").unwrap();
        let prepared = load_config(&config_path).unwrap();
        let prepared_service = prepared
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.bin")
            .unwrap();
        let ServiceStartCommand::ShellCommand {
            command: prepared_command,
            env: prepared_env,
            ..
        } = prepared_service.start_command.as_ref().unwrap();
        let expected_node = resolve_command_path("node", &prepared.runtime)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "node".to_string());
        assert_eq!(prepared_command[0], expected_node);
        assert!(!prepared_env.contains_key("PATH"));
    }

    #[test]
    fn install_connector_resolves_platform_native_bin_start_command() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        let source = dir.path().join("connector");
        let platform_dir = native_platform_bin_dirs().remove(0);
        fs::create_dir_all(source.join("bin").join(&platform_dir)).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "native-connector",
                "bin": {
                    "native-connector": "./bin/legacy.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            source.join("bin").join("legacy.js"),
            "console.log('legacy');\n",
        )
        .unwrap();
        fs::write(source.join("native-connector"), "#!/bin/sh\n").unwrap();
        fs::write(
            source
                .join("bin")
                .join(&platform_dir)
                .join("native-connector"),
            "#!/bin/sh\n",
        )
        .unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest_with_command(
                "com.baijimu.connector.native",
                "Native Connector",
                "0.1.0",
                "native-connector",
                &["start"],
            ))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        let package_path = PathBuf::from(result.package_path);
        let config = load_config(&config_path).unwrap();
        let service = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.native")
            .unwrap();
        let ServiceStartCommand::ShellCommand { command, cwd, .. } =
            service.start_command.as_ref().unwrap();
        assert_eq!(
            command[0],
            package_path
                .join("bin")
                .join(platform_dir)
                .join("native-connector")
                .display()
                .to_string()
        );
        assert_eq!(cwd.as_deref(), Some(package_path.to_str().unwrap()));
    }

    #[test]
    fn install_connector_prefers_configured_node_path() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        let configured_node = dir.path().join(if cfg!(windows) {
            "custom-node.exe"
        } else {
            "custom-node"
        });
        fs::write(&configured_node, "").unwrap();
        let mut agent_config = AgentConfig::example();
        agent_config.runtime.node_path = Some(configured_node.display().to_string());
        save_config(&config_path, &agent_config).unwrap();

        let source = dir.path().join("connector");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "test-connector",
                "bin": {
                    "test-connector": "./bin/start.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("bin").join("start.js"), "console.log('ok');\n").unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest_with_command(
                "com.baijimu.connector.configured-node",
                "Configured Node Connector",
                "0.1.0",
                "test-connector",
                &["--port", "18082"],
            ))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let config = load_config(&config_path).unwrap();
        let service = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.configured-node")
            .unwrap();
        let ServiceStartCommand::ShellCommand { command, env, .. } =
            service.start_command.as_ref().unwrap();
        assert_eq!(command[0], configured_node.display().to_string());
        assert!(!env.contains_key("PATH"));

        prepare_installed_connector_runtime(&config_path, "com.baijimu.connector.configured-node")
            .unwrap();
        let prepared = load_config(&config_path).unwrap();
        let prepared_service = prepared
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.configured-node")
            .unwrap();
        let ServiceStartCommand::ShellCommand {
            command: prepared_command,
            env: prepared_env,
            ..
        } = prepared_service.start_command.as_ref().unwrap();
        assert_eq!(prepared_command[0], configured_node.display().to_string());
        assert!(!prepared_env.contains_key("PATH"));
    }

    #[test]
    fn install_connector_derives_package_bin_stop_command() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "test-connector",
                "bin": {
                    "test-connector": "./bin/start.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("bin").join("start.js"), "console.log('ok');\n").unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest_with_command(
                "com.baijimu.connector.daemon",
                "Daemon Connector",
                "0.1.0",
                "test-connector",
                &["start", "--daemon", "--port", "18082"],
            ))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        let package_path = PathBuf::from(result.package_path);
        let config = load_config(&config_path).unwrap();
        let service = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.daemon")
            .unwrap();
        let ServiceStartCommand::ShellCommand { command, cwd, .. } =
            service.stop_command.as_ref().unwrap();
        assert_eq!(command[0], "node");
        assert_eq!(
            command[1],
            package_path.join("./bin/start.js").display().to_string()
        );
        assert_eq!(command[2], "stop");
        assert_eq!(command[3], "--port");
        assert_eq!(cwd.as_deref(), Some(package_path.to_str().unwrap()));
    }
