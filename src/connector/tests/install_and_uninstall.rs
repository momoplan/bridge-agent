    #[test]
    fn replacement_requires_a_readable_install_record() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let app_id = "test-record";
        let package_manifest = source.join(CONNECTOR_MANIFEST_FILE);
        let document = test_manifest(app_id, "Record Test", "1.0.0");
        fs::write(&package_manifest, document.to_string()).unwrap();
        install_connector_from_path(&source, &config_path, false).unwrap();
        let record_path = install_record_path(app_id).unwrap();
        let original_config = fs::read(&config_path).unwrap();
        fs::write(&record_path, b"{}").unwrap();
        assert!(install_connector_from_path(&source, &config_path, true).is_err());
        assert_eq!(fs::read(&record_path).unwrap(), b"{}");
        fs::remove_file(&record_path).unwrap();
        assert!(install_connector_from_path(&source, &config_path, true).is_err());
        assert_eq!(fs::read(&config_path).unwrap(), original_config);
    }

    #[test]
    fn install_connector_updates_agent_config() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        let mut document = test_manifest_with_command(
            "com.baijimu.connector.inline",
            "Inline Connector",
            "0.1.0",
            "connector-test",
            &["start"],
        );
        document["hostRequirements"] = json!({
            "minimumVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": ["connector.asset-upload.v1"]
        });
        document["permissions"] = json!([{ "id": "assets.upload", "title": "Upload assets" }]);
        document["transport"] = json!({ "type": "http", "baseUrl": "http://127.0.0.1:18082" });
        document["methods"] =
            json!([{ "name": "invoke", "description": "Invoke.", "path": "/invoke" }]);
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&document).unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        assert_eq!(result.app_id, "com.baijimu.connector.inline");
        assert_eq!(result.method_names, vec!["invoke".to_string()]);
        let config = load_config(&config_path).unwrap();
        assert!(config
            .local_apps
            .iter()
            .any(|app| app.app_id == "com.baijimu.connector.inline"));
        let app = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.inline")
            .unwrap();
        let crate::config::ServiceStartCommand::ShellCommand { env, .. } =
            app.start_command.as_ref().unwrap();
        assert_eq!(
            env.get(LOCAL_APP_ASSET_UPLOAD_ENDPOINT_ENV)
                .map(String::as_str),
            Some(DEFAULT_LOCAL_APP_ASSET_UPLOAD_ENDPOINT)
        );
        let token_path = env
            .get(LOCAL_APP_ASSET_UPLOAD_TOKEN_FILE_ENV)
            .map(PathBuf::from)
            .unwrap();
        assert!(
            !token_path.exists(),
            "install must not prepare Connector runtime credentials"
        );
        assert!(!config
            .services
            .iter()
            .any(|service| service.name == "inlineService"));

        prepare_installed_connector_runtime(&config_path, "com.baijimu.connector.inline").unwrap();
        assert!(token_path.is_file());
    }

    #[test]
    fn schema_v3_installs_one_local_app_without_runtime_service() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector-v2");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.v2-test",
                "name": "V2 Connector",
                "version": "1.0.0",
                "runtime": {
                    "type": "process",
                    "command": "connector-v2",
                    "args": ["start", "--daemon"]
                },
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:18082"
                },
                "methods": [{
                    "name": "invoke",
                    "description": "Invoke.",
                    "path": "/invoke"
                }],
                "events": [{
                    "name": "changed",
                    "description": "Changed."
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        assert_eq!(result.app_id, "com.baijimu.connector.v2-test");
        assert_eq!(result.method_names, vec!["invoke".to_string()]);
        assert_eq!(result.event_names, vec!["changed".to_string()]);

        let config = load_config(&config_path).unwrap();
        assert_eq!(
            config
                .local_apps
                .iter()
                .filter(|app| app.app_id == "com.baijimu.connector.v2-test")
                .count(),
            1
        );
        assert_eq!(
            config
                .services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>(),
            vec!["computer", "shell", "files"]
        );

        let record = load_install_record("com.baijimu.connector.v2-test").unwrap();
        assert_eq!(record.manifest.app_id, "com.baijimu.connector.v2-test");
    }

    #[test]
    fn uninstall_connector_removes_package_and_install_record() {
        let dir = tempdir().unwrap();
        let connectors_dir = dir.path().join("connectors");
        let _env = connector_test_env(connectors_dir.clone());
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest(
                "com.baijimu.connector.uninstall-test",
                "Uninstall Test Connector",
                "0.1.0",
            ))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let install_root = connectors_dir.join("com.baijimu.connector.uninstall-test");
        assert!(install_root.join("package").is_dir());
        assert!(install_root.join(CONNECTOR_INSTALL_RECORD_FILE).is_file());

        uninstall_connector("com.baijimu.connector.uninstall-test", &config_path).unwrap();

        assert!(!install_root.exists());
        assert!(list_connectors().unwrap().is_empty());
        assert!(!load_config(&config_path)
            .unwrap()
            .local_apps
            .iter()
            .any(|app| app.app_id == "com.baijimu.connector.uninstall-test"));
    }

    #[test]
    fn forced_uninstall_is_explicit_and_platform_safe_after_stop_failure() {
        let dir = tempdir().unwrap();
        let connectors_dir = dir.path().join("connectors");
        let _env = connector_test_env(connectors_dir.clone());
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest(
                "com.baijimu.connector.force-uninstall-test",
                "Force Uninstall Test Connector",
                "0.1.0",
            ))
            .unwrap(),
        )
        .unwrap();
        install_connector_from_path(&source, &config_path, false).unwrap();

        let mut config = load_config(&config_path).unwrap();
        let app = config
            .local_apps
            .iter_mut()
            .find(|app| app.app_id == "com.baijimu.connector.force-uninstall-test")
            .unwrap();
        app.start_command = Some(failing_test_command());
        app.stop_command = Some(failing_test_command());
        save_config(&config_path, &config).unwrap();

        let install_root = connectors_dir.join("com.baijimu.connector.force-uninstall-test");
        #[cfg(unix)]
        let mut package_worker = {
            let worker = install_root.join("package").join("worker.sh");
            fs::write(&worker, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
            let mut permissions = fs::metadata(&worker).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&worker, permissions).unwrap();
            Command::new(&worker).spawn().unwrap()
        };

        let error = uninstall_connector("com.baijimu.connector.force-uninstall-test", &config_path)
            .unwrap_err();
        assert!(error.to_string().contains("failed to stop connector"));
        assert!(is_connector_package_stop_error(&error));
        assert!(install_root.exists());

        let forced = uninstall_connector_with_options(
            "com.baijimu.connector.force-uninstall-test",
            &config_path,
            ConnectorUninstallOptions { force: true },
        );
        #[cfg(windows)]
        {
            forced.unwrap();
            assert!(!install_root.exists());
        }
        #[cfg(unix)]
        {
            forced.unwrap();
            assert!(!install_root.exists());
            assert!(!package_worker.wait().unwrap().success());
        }
        #[cfg(all(not(windows), not(unix)))]
        {
            let error = forced.unwrap_err();
            assert!(error.to_string().contains("unsupported on this platform"));
            assert!(install_root.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_process_inspection_matches_only_the_connector_package_tree() {
        let package_path = Path::new("/tmp/Bridge Agent/connectors/example/package");
        let process_list = "\
  101 /tmp/Bridge Agent/connectors/example/package/bin/worker --daemon\n\
  102 /usr/bin/python /tmp/Bridge Agent/connectors/example/package/service.py\n\
  103 /tmp/Bridge Agent/connectors/example/package-other/bin/worker\n\
  104 /bin/sh -c echo /tmp/Bridge Agent/connectors/example/package\n\
  105 /usr/bin/unrelated\n";

        assert_eq!(
            parse_unix_connector_package_processes(process_list, package_path, 102),
            vec![101]
        );
    }

    #[test]
    fn bound_method_uses_current_frozen_installation_and_registered_target() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let package = dir.path().join("fixture");
        fs::create_dir(&package).unwrap();
        let app_id = "com.example.bound";
        fs::write(package.join(CONNECTOR_MANIFEST_FILE), test_manifest(app_id, "fixture", "1.2.0").to_string()).unwrap();
        install_connector_from_path(&package, &config_path, false).unwrap();
        prepare_installed_connector_runtime(&config_path, app_id).unwrap();
        let mut config = load_config(&config_path).unwrap();
        let mut record = show_connector(app_id).unwrap();
        let source: local_app_contract::SourceVersion = serde_json::from_value(json!({
            "application":{"environmentKey":"private","appId":app_id},"version":"1.2.0"
        })).unwrap();
        let get_registry = |cfg: &AgentConfig| crate::services::ServiceRegistry::from_config(cfg,dir.path()).unwrap();
        assert!(get_registry(&config).bound_app_id(&source, "ping").is_err());
        record.install_source = Some(local_app_contract::InstallSource::Environment {source:source.clone()});
        save_install_record(&record).unwrap();
        let registry = get_registry(&config);
        assert_eq!(registry.bound_app_id(&source, "ping").unwrap(), app_id);
        assert!(registry.bound_app_id(&source, "undeclared").is_err());
        let other = local_app_contract::SourceVersion {
            application: local_app_contract::SourceApp {environment_key:"other".to_string().try_into().unwrap(), ..source.application.clone()},
            ..source.clone()
        };
        assert!(registry.bound_app_id(&other, "ping").is_err());
        record.install_source = Some(local_app_contract::InstallSource::Environment {source:other});
        save_install_record(&record).unwrap();
        assert!(registry.bound_app_id(&source, "ping").is_err());
        record.install_source = Some(local_app_contract::InstallSource::Environment {source:source.clone()});
        save_install_record(&record).unwrap();
        let app = config.local_apps.iter_mut().find(|app|app.app_id == app_id).unwrap();
        let crate::config::MethodBinding::Http(binding) = &mut app.methods[0].binding else { panic!("fixture is HTTP") };
        binding.url.push_str("/unexpected");
        assert!(get_registry(&config).bound_app_id(&source, "ping").is_err());
    }
