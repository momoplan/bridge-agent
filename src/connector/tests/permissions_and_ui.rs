    #[test]
    fn host_managed_connector_start_requires_desktop_supervisor() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.host-start",
                "name": "Host Start Connector",
                "version": "1.0.0",
                "runtime": {
                    "type": "process",
                    "command": "host-start-connector",
                    "args": ["run"],
                    "stopArgs": ["stop"],
                    "processOwnership": "host"
                },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }],
                "hostRequirements": {
                    "minimumVersion": env!("CARGO_PKG_VERSION"),
                    "capabilities": ["connector.process.host-managed.v1"]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        install_connector_from_path(&source, &config_path, false).unwrap();

        let error = start_connector("com.baijimu.connector.host-start", &config_path).unwrap_err();
        assert!(error.to_string().contains("desktop process supervisor"));
    }

    #[test]
    fn connector_manifest_accepts_existing_camel_case_permission_id() {
        let manifest: ConnectorManifest = serde_json::from_value(json!({
            "schemaVersion": "3.0.0",
            "appId": "com.baijimu.connector.wecom",
            "name": "WeCom Connector",
            "version": "1.0.2",
            "runtime": { "type": "process", "command": "wecom-bridge-collector-python" },
            "permissions": [{
                "id": "macos.fullDiskAccess",
                "title": "完全磁盘访问",
                "platforms": ["macos"]
            }],
            "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18084" },
            "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
        }))
        .unwrap();

        validate_manifest(&manifest).unwrap();
    }

    #[test]
    fn connector_manifest_rejects_permission_ids_outside_ascii_contract() {
        for permission_id in [
            "macos.full disk access",
            "macos.完全磁盘访问",
            "macos.fullDiskAccess/legacy",
        ] {
            let manifest: ConnectorManifest = serde_json::from_value(json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.invalid-permission",
                "name": "Invalid Permission Connector",
                "version": "1.0.0",
                "runtime": { "type": "process", "command": "invalid-permission" },
                "permissions": [{ "id": permission_id, "title": "Invalid" }],
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
            }))
            .unwrap();

            let error = validate_manifest(&manifest).unwrap_err();
            assert!(error
                .to_string()
                .contains("permission id must use ASCII letters"));
        }
    }

    #[test]
    fn connector_manifest_accepts_embedded_ui_and_resolves_only_ui_assets() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ui/assets")).unwrap();
        fs::write(dir.path().join("ui/index.html"), "<main>settings</main>").unwrap();
        fs::write(dir.path().join("ui/assets/app.js"), "window.loaded = true;").unwrap();
        fs::write(dir.path().join("private.txt"), "not a UI asset").unwrap();
        let mut document = test_manifest(
            "com.baijimu.connector.with-ui",
            "Connector With UI",
            "0.1.0",
        );
        document["icon"] = test_connector_icon();
        document["ui"] = json!({
            "type": "embedded",
            "entry": "ui/index.html",
            "title": "个性化设置",
            "defaultView": true
        });
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&document).unwrap(),
        )
        .unwrap();

        let manifest = load_connector_manifest(dir.path()).unwrap();
        assert!(connector_icon_data_url(manifest.icon.as_ref().unwrap())
            .unwrap()
            .starts_with("data:image/png;base64,"));
        let ui = manifest.ui.unwrap();
        assert!(ui.default_view);
        assert_eq!(ui.title.as_deref(), Some("个性化设置"));
        assert_eq!(
            resolve_connector_ui_entry(dir.path(), &ui).unwrap(),
            dir.path().join("ui/index.html").canonicalize().unwrap()
        );
        assert_eq!(
            resolve_connector_ui_asset(dir.path(), &ui, Some("assets/app.js")).unwrap(),
            dir.path().join("ui/assets/app.js").canonicalize().unwrap()
        );
        assert!(resolve_connector_ui_asset(dir.path(), &ui, Some("../private.txt")).is_err());
    }

    #[test]
    fn connector_manifest_rejects_invalid_icon_contracts() {
        let base = test_manifest(
            "com.baijimu.connector.bad-icon",
            "Bad Icon Connector",
            "0.1.0",
        );

        let mut wrong_media_type = base.clone();
        wrong_media_type["icon"] = test_connector_icon();
        wrong_media_type["icon"]["mediaType"] = json!("image/svg+xml");
        assert!(
            validate_manifest(&serde_json::from_value(wrong_media_type).unwrap())
                .unwrap_err()
                .to_string()
                .contains("icon.mediaType")
        );

        let mut malformed = base.clone();
        malformed["icon"] = json!({ "mediaType": "image/png", "data": "not-base64" });
        assert!(
            validate_manifest(&serde_json::from_value(malformed).unwrap())
                .unwrap_err()
                .to_string()
                .contains("valid base64")
        );

        let image = RgbaImage::from_pixel(128, 128, Rgba([14, 122, 80, 255]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let mut wrong_size = base;
        wrong_size["icon"] = json!({
            "mediaType": "image/png",
            "data": BASE64_STANDARD.encode(bytes.into_inner())
        });
        assert!(
            validate_manifest(&serde_json::from_value(wrong_size).unwrap())
                .unwrap_err()
                .to_string()
                .contains("256x256")
        );
    }

    #[test]
    fn connector_manifest_rejects_missing_or_escaping_ui_entry() {
        let dir = tempdir().unwrap();
        let manifest_path = dir.path().join(CONNECTOR_MANIFEST_FILE);
        let base = test_manifest("com.baijimu.connector.bad-ui", "Bad UI Connector", "0.1.0");

        let mut missing = base.clone();
        missing["ui"] = json!({ "type": "embedded", "entry": "ui/missing.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&missing).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("failed to resolve connector UI root"));

        let mut escaping = base.clone();
        escaping["ui"] = json!({ "type": "embedded", "entry": "../outside.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&escaping).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("must stay inside"));

        fs::write(dir.path().join("index.html"), "<main>root UI</main>").unwrap();
        let mut root_entry = base.clone();
        root_entry["ui"] = json!({ "type": "embedded", "entry": "index.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&root_entry).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("dedicated UI directory"));
    }

    #[test]
    fn installed_connector_summary_exposes_embedded_ui_metadata() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("installed-connectors"));
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("ui")).unwrap();
        fs::write(
            source.join("ui/index.html"),
            "<!doctype html><title>UI</title>",
        )
        .unwrap();
        let mut document = test_manifest(
            "com.baijimu.connector.installed-ui",
            "Installed UI Connector",
            "0.1.0",
        );
        document["icon"] = test_connector_icon();
        document["ui"] = json!({
            "type": "embedded",
            "entry": "ui/index.html",
            "title": "设置",
            "defaultView": true
        });
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&document).unwrap(),
        )
        .unwrap();
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let summary = list_connectors().unwrap().remove(0);
        let serialized = serde_json::to_value(&summary).unwrap();
        assert_eq!(serialized["ui"]["type"], "embedded");
        assert!(serialized["ui"].get("uiType").is_none());
        assert_eq!(serialized["ui"]["defaultView"], true);
        assert!(serialized["iconDataUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        let ui = summary.ui.expect("installed UI metadata");
        assert_eq!(ui.ui_type, "embedded");
        assert_eq!(ui.entry, "ui/index.html");
        assert_eq!(ui.title.as_deref(), Some("设置"));
        assert!(ui.default_view);
        assert_eq!(summary.review_status, "DRAFT");
        assert!(summary
            .package_checksum
            .as_deref()
            .is_some_and(|value| value.starts_with("sha256:") && value.len() == 71));

        let market_checksum = format!("sha256:{}", "a".repeat(64));
        let provenance = ConnectorInstallProvenance::registered(
            "https://downloads.example.test/connector.zip",
            "PUBLISHED",
            &market_checksum,
        )
        .unwrap();
        install_connector_from_path_with_provenance(&source, &config_path, true, provenance)
            .unwrap();
        let trusted = list_connectors().unwrap().remove(0);
        assert_eq!(trusted.app_id, "com.baijimu.connector.installed-ui");
        assert_eq!(trusted.review_status, "PUBLISHED");
        assert_eq!(
            trusted.source_checksum.as_deref(),
            Some(market_checksum.as_str())
        );

        assert_eq!(
            show_connector(&trusted.app_id).unwrap().manifest.app_id,
            trusted.app_id
        );
    }

    #[test]
    fn installed_connector_summary_preserves_upgrade_review_contracts() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("installed-connectors"));
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.review-contract",
                "name": "Review Contract Connector",
                "version": "1.0.0",
                "runtime": { "type": "process", "command": "review-contract" },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "configSchema": {"type": "object", "required": ["token"], "properties": {"token": {"type": "string"}}},
                "methods": [{
                    "name": "ping",
                    "description": "Ping.",
                    "path": "/invoke/ping",
                    "httpMethod": "POST",
                    "input_schema": {"type": "object", "properties": {"value": {"type": "string"}}}
                }],
                "database": {
                    "engine": "sqlite",
                    "schemaVersion": "2",
                    "migrations": [{
                        "id": "002-add-status",
                        "fromVersion": "1",
                        "toVersion": "2",
                        "description": "Add status",
                        "changes": [{"operation": "add_column", "target": "items.status", "description": "Add status"}],
                        "rollback": "automatic",
                        "downtime": "none"
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let summary = list_connectors().unwrap().remove(0);

        assert_eq!(
            summary.config_schema.as_ref().unwrap()["required"][0],
            "token"
        );
        assert_eq!(summary.methods[0].path, "/invoke/ping");
        assert_eq!(
            summary.methods[0].input_schema["properties"]["value"]["type"],
            "string"
        );
        assert_eq!(summary.database.as_ref().unwrap().schema_version, "2");
    }

    #[test]
    fn registered_provenance_requires_review_status_source_and_sha256_evidence() {
        assert!(ConnectorInstallProvenance::registered(
            "https://downloads.example.test/connector.zip",
            "PUBLISHED",
            "invalid",
        )
        .is_err());
        assert!(ConnectorInstallProvenance::registered("", "PUBLISHED", &"a".repeat(64)).is_err());
        assert!(ConnectorInstallProvenance::registered(
            "https://downloads.example.test/connector.zip",
            "",
            &"a".repeat(64),
        )
        .is_err());
    }

    #[test]
    fn connector_manifest_rejects_remote_management_url() {
        let dir = tempdir().unwrap();
        let mut document =
            test_manifest("com.baijimu.connector.unsafe", "Unsafe Connector", "0.1.0");
        document["management"] = json!({
            "type": "http",
            "baseUrl": "https://example.com",
            "auth": { "type": "connector_token" },
            "operations": { "state": { "method": "GET", "path": "/management/v1/state" } }
        });
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&document).unwrap(),
        )
        .unwrap();

        let error = load_connector_manifest(dir.path()).unwrap_err();
        assert!(error.to_string().contains("management.baseUrl"));
    }

    #[test]
    fn connector_manifest_rejects_management_base_url_with_path() {
        let management = ConnectorManagement {
            management_type: "http".to_string(),
            base_url: "http://127.0.0.1:18110/untrusted".to_string(),
            auth: ConnectorManagementAuth {
                auth_type: "connector_token".to_string(),
            },
            operations: BTreeMap::from([(
                "state".to_string(),
                ConnectorManagementOperation {
                    method: "GET".to_string(),
                    path: "/management/v1/state".to_string(),
                },
            )]),
        };

        let error = validate_management(&management).unwrap_err();
        assert!(error.to_string().contains("origin-only"));
    }
