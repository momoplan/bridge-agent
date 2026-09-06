    fn test_connector_icon() -> Value {
        let image = RgbaImage::from_pixel(
            CONNECTOR_ICON_EDGE_PX,
            CONNECTOR_ICON_EDGE_PX,
            Rgba([14, 122, 80, 255]),
        );
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        json!({
            "mediaType": CONNECTOR_ICON_MEDIA_TYPE,
            "data": BASE64_STANDARD.encode(bytes.into_inner())
        })
    }

    fn test_manifest(app_id: &str, name: &str, version: &str) -> Value {
        #[cfg(windows)]
        let (command, args): (&str, &[&str]) = ("cmd.exe", &["/C", "exit", "0"]);
        #[cfg(not(windows))]
        let (command, args): (&str, &[&str]) = ("/usr/bin/true", &["--"]);
        let mut manifest = test_manifest_with_command(app_id, name, version, command, args);
        manifest["runtime"]["stopArgs"] = json!(args);
        manifest
    }

    fn test_manifest_with_command(
        app_id: &str,
        name: &str,
        version: &str,
        command: &str,
        args: &[&str],
    ) -> Value {
        json!({
            "schemaVersion": "3.0.0",
            "appId": app_id,
            "name": name,
            "version": version,
            "runtime": { "type": "process", "command": command, "args": args },
            "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
            "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
        })
    }

    #[test]
    fn local_app_automatic_start_requires_enabled_lifecycle_command() {
        let mut app = LocalAppConfig {
            app_id: "com.baijimu.connector.test".to_string(),
            name: "Test".to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            enabled: true,
            health_check: None,
            start_command: Some(ServiceStartCommand::ShellCommand {
                command: vec!["test-connector".to_string(), "start".to_string()],
                cwd: None,
                env: BTreeMap::from([(
                    LOCAL_APP_START_POLICY_ENV.to_string(),
                    "automatic".to_string(),
                )]),
                timeout_secs: None,
            }),
            stop_command: None,
            methods: Vec::new(),
            events: Vec::new(),
        };

        assert!(local_app_starts_automatically(&app));
        app.enabled = false;
        assert!(!local_app_starts_automatically(&app));
        app.enabled = true;
        let Some(ServiceStartCommand::ShellCommand { env, .. }) = app.start_command.as_mut() else {
            panic!("expected shell start command");
        };
        env.insert(LOCAL_APP_START_POLICY_ENV.to_string(), "manual".to_string());
        assert!(!local_app_starts_automatically(&app));
        app.start_command = None;
        assert!(!local_app_starts_automatically(&app));
    }

    struct ConnectorTestEnvGuard {
        _lock: MutexGuard<'static, ()>,
        previous: Option<OsString>,
        previous_data: Option<OsString>,
    }

    impl Drop for ConnectorTestEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("BRIDGE_AGENT_LOCAL_APPS_DIR", previous);
            } else {
                std::env::remove_var("BRIDGE_AGENT_LOCAL_APPS_DIR");
            }
            if let Some(previous) = self.previous_data.as_ref() {
                std::env::set_var("BRIDGE_AGENT_LOCAL_APP_DATA_DIR", previous);
            } else {
                std::env::remove_var("BRIDGE_AGENT_LOCAL_APP_DATA_DIR");
            }
        }
    }

    fn connector_test_env(path: impl AsRef<Path>) -> ConnectorTestEnvGuard {
        let lock = CONNECTOR_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("BRIDGE_AGENT_LOCAL_APPS_DIR");
        let previous_data = std::env::var_os("BRIDGE_AGENT_LOCAL_APP_DATA_DIR");
        std::env::set_var("BRIDGE_AGENT_LOCAL_APPS_DIR", path.as_ref());
        std::env::set_var(
            "BRIDGE_AGENT_LOCAL_APP_DATA_DIR",
            path.as_ref().join("data"),
        );
        ConnectorTestEnvGuard {
            _lock: lock,
            previous,
            previous_data,
        }
    }
