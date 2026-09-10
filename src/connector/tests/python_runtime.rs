    #[test]
    fn reads_python_project_scripts() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[project.scripts]
wechat-bridge-collector = "wechat_bridge_collector.app:main"
"#,
        )
        .unwrap();

        let scripts = read_python_project_scripts(dir.path()).unwrap();
        assert_eq!(
            scripts.get("wechat-bridge-collector").map(String::as_str),
            Some("wechat_bridge_collector.app:main")
        );
    }

    #[test]
    fn writes_python_project_script_for_source_tree() {
        let dir = tempdir().unwrap();
        let package_path = dir.path().join("package");
        let env_path = package_path.join(CONNECTOR_PYTHON_ENV_DIR);
        fs::create_dir_all(python_bin_dir(&env_path)).unwrap();
        fs::create_dir_all(package_path.join("sample_connector")).unwrap();
        fs::write(
            package_path.join("sample_connector").join("app.py"),
            "def run():\n    print('ok')\n    return 0\n",
        )
        .unwrap();

        write_python_project_script(
            &package_path,
            &env_path,
            "sample-connector",
            "sample_connector.app:run",
        )
        .unwrap();

        let script = fs::read_to_string(python_script_path(&env_path, "sample-connector")).unwrap();
        assert!(script.contains("sys.path.insert"));
        assert!(script.contains("sample_connector.app"));
        assert!(script.contains("run()"));
    }

    #[test]
    fn resolves_python_project_script_to_connector_environment() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(CONNECTOR_PYTHON_ENV_DIR);
        fs::create_dir_all(python_bin_dir(&env_path)).unwrap();
        let mut command = vec!["wechat-bridge-collector".to_string(), "start".to_string()];
        let mut cwd = None;
        let mut env_vars = BTreeMap::new();
        let scripts = BTreeMap::from([(
            "wechat-bridge-collector".to_string(),
            "wechat_bridge_collector.app".to_string(),
        )]);

        let command_runtime = InstalledCommandRuntime {
            package_path: dir.path(),
            package_bins: &BTreeMap::new(),
            python_scripts: &scripts,
            python_env: Some(&env_path),
            node_path: &None,
        };
        resolve_installed_shell_command(&mut command, &mut cwd, &mut env_vars, &command_runtime);

        assert_eq!(
            command[0],
            python_script_path(&env_path, "wechat-bridge-collector")
                .display()
                .to_string()
        );
        assert_eq!(command[1], "start");
        assert_eq!(cwd.as_deref(), Some(dir.path().to_str().unwrap()));
        assert!(env_vars
            .get("PATH")
            .is_some_and(|path| env::split_paths(path)
                .next()
                .is_some_and(|entry| entry == python_bin_dir(&env_path))));
    }

    #[test]
    fn windows_command_resolution_ignores_extensionless_npm_shim() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("codex"), "#!/bin/sh\n").unwrap();
        let windows_launcher = dir.path().join("codex.cmd");
        fs::write(&windows_launcher, "@echo off\r\n").unwrap();
        let path = env::join_paths([dir.path()])
            .unwrap()
            .to_string_lossy()
            .to_string();

        let resolved = find_command_in_path_for_platform(
            "codex",
            Some(&path),
            true,
            &[".exe".to_string(), ".cmd".to_string()],
        );

        assert_eq!(resolved.as_deref(), Some(windows_launcher.as_path()));
    }

    #[test]
    fn windows_command_resolution_rejects_extensionless_only_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("codex"), "#!/bin/sh\n").unwrap();

        assert_eq!(
            resolve_command_file(
                &dir.path().join("codex"),
                true,
                &[".exe".to_string(), ".cmd".to_string()],
            ),
            None
        );
    }

    #[test]
    fn windows_command_extensions_exclude_non_native_script_types() {
        assert_eq!(
            windows_command_extensions_from(Some(".JS;.CMD;.cmd;.EXE;.PS1")),
            vec![".CMD".to_string(), ".EXE".to_string()]
        );
    }

    #[test]
    fn windows_command_resolution_rejects_unsupported_explicit_script() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("codex.ps1");
        fs::write(&script, "Write-Output codex\n").unwrap();

        assert_eq!(
            resolve_command_file(&script, true, &[".exe".to_string(), ".cmd".to_string()]),
            None
        );
    }

    #[test]
    fn parses_python_version_requirements() {
        let python_312 = parse_python_version("Python 3.12.7").unwrap();
        let python_314 = parse_python_version("3.14.4").unwrap();
        assert_eq!(python_312.to_string(), "3.12.7");
        assert_eq!(parse_python_version("3.10").unwrap().to_string(), "3.10");
        assert!(python_version_matches_requirement(&python_314, None));

        let connector_312 = VersionSpecifiers::from_str(">=3.12,<3.13").unwrap();
        assert!(python_version_matches_requirement(
            &python_312,
            Some(&connector_312)
        ));
        assert!(!python_version_matches_requirement(
            &python_314,
            Some(&connector_312)
        ));

        let connector_314 = VersionSpecifiers::from_str(">=3.14").unwrap();
        assert!(connector_314.contains(&python_314));

        let compatible_release = VersionSpecifiers::from_str("~=3.12.0,!=3.12.5").unwrap();
        assert!(compatible_release.contains(&python_312));
        assert!(!compatible_release.contains(&Version::from_str("3.12.5").unwrap()));
    }

    #[test]
    fn recognizes_versioned_python_executables_without_imposing_a_version() {
        for name in [
            "python",
            "python3",
            "python3.9",
            "python3.12",
            "python3.14.exe",
            "Python4.0.EXE",
        ] {
            assert!(is_python_executable_name(name), "expected {name} to match");
        }
        for name in [
            "pythonw.exe",
            "python-config",
            "python3.12-config",
            "ipython",
        ] {
            assert!(
                !is_python_executable_name(name),
                "expected {name} not to match"
            );
        }
    }

    #[test]
    fn parses_all_windows_py_launcher_paths() {
        let paths = parse_windows_python_launcher_paths(
            " -V:3.14 *        C:\\Users\\50165\\AppData\\Local\\Programs\\Python\\Python314\\python.exe\r\n\
             -V:3.12          C:\\Users\\Example User\\AppData\\Local\\Programs\\Python\\Python312\\python.exe\r\n",
        );

        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"C:\Users\50165\AppData\Local\Programs\Python\Python314\python.exe"),
                PathBuf::from(
                    r"C:\Users\Example User\AppData\Local\Programs\Python\Python312\python.exe"
                ),
            ]
        );
    }

    #[test]
    fn sync_installed_connectors_restores_lifecycle_commands_and_preserves_enabled() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest_with_command(
                "com.baijimu.connector.sync",
                "Sync Connector",
                "0.1.0",
                "/bin/sh",
                &["-c", "sleep 60"],
            ))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let mut config = load_config(&config_path).unwrap();
        let service = config
            .local_apps
            .iter_mut()
            .find(|app| app.app_id == "com.baijimu.connector.sync")
            .unwrap();
        service.enabled = false;
        service.start_command = None;
        save_config(&config_path, &config).unwrap();

        sync_installed_connectors(&config_path).unwrap();
        let config = load_config(&config_path).unwrap();
        let service = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.sync")
            .unwrap();
        assert!(!service.enabled);
        assert!(service.start_command.is_some());

        let mut config = load_config(&config_path).unwrap();
        config
            .local_apps
            .retain(|app| app.app_id != "com.baijimu.connector.sync");
        save_config(&config_path, &config).unwrap();

        sync_installed_connector(&config_path, "com.baijimu.connector.sync").unwrap();
        let config = load_config(&config_path).unwrap();
        let restored = config
            .local_apps
            .iter()
            .find(|app| app.app_id == "com.baijimu.connector.sync")
            .expect("installed connector must restore missing derived config");
        assert!(restored.enabled);
        assert!(restored.start_command.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn install_and_catalog_sync_do_not_execute_python_runtime() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let marker = dir.path().join("python-invoked");
        let fake_python = dir.path().join("python3");
        fs::write(
            &fake_python,
            format!(
                "#!/bin/sh\ntouch '{}'\nif [ \"$1\" = \"--version\" ]; then echo 'Python 3.12.1'; exit 0; fi\nexit 1\n",
                marker.display()
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake_python).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_python, permissions).unwrap();

        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.runtime.python_path = Some(fake_python.display().to_string());
        save_config(&config_path, &config).unwrap();

        let source = dir.path().join("python-connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest_with_command(
                "com.baijimu.connector.lazy-python",
                "Lazy Python Connector",
                "0.1.0",
                "lazy-python-connector",
                &["start"],
            ))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            source.join("pyproject.toml"),
            r#"[project]
name = "lazy-python-connector"
version = "0.1.0"
requires-python = ">=3.12,<3.13"

[project.scripts]
lazy-python-connector = "lazy_python_connector.app:main"
"#,
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        sync_installed_connectors(&config_path).unwrap();
        assert!(!marker.exists(), "install and catalog sync executed Python");

        let error =
            prepare_installed_connector_runtime(&config_path, "com.baijimu.connector.lazy-python")
                .unwrap_err();
        assert!(
            marker.exists(),
            "runtime preparation did not execute Python"
        );
        assert!(error
            .to_string()
            .contains("failed to create Python connector environment"));
    }

    #[test]
    fn sync_defers_python_runtime_validation_until_connector_start() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        let good_source = dir.path().join("good-connector");
        fs::create_dir_all(&good_source).unwrap();
        fs::write(
            good_source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&test_manifest(
                "com.baijimu.connector.good",
                "Good Connector",
                "0.1.0",
            ))
            .unwrap(),
        )
        .unwrap();
        install_connector_from_path(&good_source, &config_path, false).unwrap();

        let bad_manifest: ConnectorManifest = serde_json::from_value(test_manifest_with_command(
            "com.baijimu.connector.bad-python",
            "Bad Python Connector",
            "0.1.0",
            "bad-python-connector",
            &["start"],
        ))
        .unwrap();
        let bad_package_path = installed_connector_package_path(&bad_manifest).unwrap();
        fs::create_dir_all(&bad_package_path).unwrap();
        fs::write(
            bad_package_path.join("pyproject.toml"),
            r#"[project]
name = "bad-python-connector"
version = "0.1.0"
requires-python = ">=999.0"

[project.scripts]
bad-python-connector = "bad_python_connector.app:main"
"#,
        )
        .unwrap();
        save_install_record(&ConnectorInstallRecord {
            install_source: None,
            manifest: bad_manifest,
            package_path: bad_package_path.display().to_string(),
            source_path: dir
                .path()
                .join("bad-connector-source")
                .display()
                .to_string(),
            source_reference: None,
            review_status: "DRAFT".to_string(),
            source_checksum: None,
            package_checksum: None,
            installed_at_epoch_ms: 1,
            last_synced_at_epoch_ms: 1,
        })
        .unwrap();

        let report = sync_installed_connectors_report(&config_path).unwrap();
        assert!(report
            .summaries
            .iter()
            .any(|summary| summary.app_id == "com.baijimu.connector.good"));
        assert!(report
            .summaries
            .iter()
            .any(|summary| summary.app_id == "com.baijimu.connector.bad-python"));
        assert!(report.failures.is_empty());

        sync_installed_connectors(&config_path).unwrap();
        let runtime_error =
            prepare_installed_connector_runtime(&config_path, "com.baijimu.connector.bad-python")
                .unwrap_err();
        assert!(runtime_error.to_string().contains(
            "failed to find a Python interpreter matching Connector requires-python `>=999.0`"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_python_discovery_rejects_apple_developer_tool_shims() {
        assert!(!discovered_python_candidate_is_allowed(Path::new(
            "/usr/bin/python3"
        )));
        assert!(discovered_python_candidate_is_allowed(Path::new(
            "/opt/homebrew/bin/python3"
        )));
    }

    #[test]
    fn legacy_autostart_cleanup_uses_manifest_labels() {
        let manifest = ConnectorManifest {
            schema_version: "3.0.0".to_string(),
            app_id: "com.baijimu.connector.wechat".to_string(),
            name: "WeChat Connector".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            publisher: None,
            source: None,
            runtime: None,
            management: None,
            setup: None,
            host_requirements: None,
            managed_tool_dependencies: Vec::new(),
            icon: None,
            ui: None,
            config_schema: None,
            upgrade_review: None,
            database: None,
            remote_capabilities: Vec::new(),
            permissions: Vec::new(),
            legacy_autostart_labels: Vec::new(),
            transport: None,
            methods: Vec::new(),
            events: Vec::new(),
            hooks: BTreeMap::from([(
                "installAutostart".to_string(),
                "wechat-bridge-collector install-autostart".to_string(),
            )]),
        };

        assert_eq!(
            legacy_autostart_labels_for_manifest(&manifest),
            vec![
                "com.baijimu.wechat-bridge-collector".to_string(),
                "com.baijimu.connector.wechat".to_string(),
            ]
        );
    }
