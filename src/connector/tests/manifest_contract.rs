    #[test]
    fn connector_manifest_rejects_retired_service_registration_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.test",
                "name": "Test Connector",
                "version": "0.1.0",
                "serviceRegistrationFiles": ["service-registration.json"]
            }))
            .unwrap(),
        )
        .unwrap();
        let error = load_connector_manifest(dir.path()).unwrap_err();
        assert!(format!("{error:#}").contains("serviceRegistrationFiles"));
    }

    #[test]
    fn connector_manifest_accepts_loopback_management_operations() {
        let dir = tempdir().unwrap();
        let mut document = test_manifest(
            "com.baijimu.connector.managed",
            "Managed Connector",
            "0.1.0",
        );
        document["management"] = json!({
            "type": "http",
            "baseUrl": "http://127.0.0.1:18110",
            "auth": { "type": "connector_token" },
            "operations": { "state": { "method": "GET", "path": "/management/v1/state" } }
        });
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&document).unwrap(),
        )
        .unwrap();

        let manifest = load_connector_manifest(dir.path()).unwrap();
        let management = manifest.management.unwrap();
        assert_eq!(management.auth.auth_type, "connector_token");
        assert_eq!(management.operations["state"].path, "/management/v1/state");
    }

    #[test]
    fn connector_manifest_accepts_one_click_setup_contract() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.setup",
                "name": "Setup Connector",
                "version": "1.0.0",
                "runtime": {
                    "type": "process",
                    "command": "setup-connector"
                },
                "management": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:18110",
                    "auth": { "type": "connector_token" },
                    "operations": {
                        "setupRetry": { "method": "POST", "path": "/management/v1/setup/retry" },
                        "setupState": { "method": "GET", "path": "/management/v1/setup/state" }
                    }
                },
                "setup": {
                    "operation": "setupRetry",
                    "statusOperation": "setupState",
                    "timeoutSecs": 900
                },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
            }))
            .unwrap(),
        )
        .unwrap();

        let manifest = load_connector_manifest(dir.path()).unwrap();
        let setup = manifest.setup.unwrap();
        assert_eq!(setup.operation, "setupRetry");
        assert_eq!(setup.status_operation, "setupState");
        assert_eq!(setup.timeout_secs, 900);
    }

    #[test]
    fn connector_manifest_rejects_setup_without_declared_status_operation() {
        let manifest: ConnectorManifest = serde_json::from_value(json!({
            "schemaVersion": "3.0.0",
            "appId": "com.baijimu.connector.bad-setup",
            "name": "Bad Setup Connector",
            "version": "1.0.0",
            "runtime": { "type": "process", "command": "bad-setup" },
            "management": {
                "type": "http",
                "baseUrl": "http://127.0.0.1:18110",
                "auth": { "type": "connector_token" },
                "operations": {
                    "setupRetry": { "method": "POST", "path": "/management/v1/setup/retry" }
                }
            },
            "setup": {
                "operation": "setupRetry",
                "statusOperation": "setupState"
            },
            "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
            "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
        }))
        .unwrap();

        let error = validate_manifest(&manifest).unwrap_err();
        assert!(error.to_string().contains("setup.statusOperation"));
    }

    #[test]
    fn connector_manifest_enforces_host_version_and_capabilities() {
        let manifest = |requirements: Value| {
            serde_json::from_value::<ConnectorManifest>(json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.host-requirements",
                "name": "Host Requirements Connector",
                "version": "1.0.0",
                "runtime": { "type": "process", "command": "example" },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }],
                "hostRequirements": requirements
            }))
            .unwrap()
        };

        let compatible = manifest(json!({
            "minimumVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": ["connector.setup.v1"]
        }));
        validate_manifest(&compatible).unwrap();

        let future = manifest(json!({ "minimumVersion": "99.0.0" }));
        assert!(validate_manifest(&future)
            .unwrap_err()
            .to_string()
            .contains("请先升级客户端"));

        let missing = manifest(json!({ "capabilities": ["connector.unknown.v1"] }));
        assert!(validate_manifest(&missing)
            .unwrap_err()
            .to_string()
            .contains("connector.unknown.v1"));
    }

    #[test]
    fn connector_manifest_validates_managed_tool_dependencies() {
        let manifest = |dependency: Value, capabilities: Value, runtime_env: Value| {
            serde_json::from_value::<ConnectorManifest>(json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.managed-tool",
                "name": "Managed Tool Connector",
                "version": "1.0.0",
                "runtime": {
                    "type": "process",
                    "command": "example",
                    "env": runtime_env
                },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }],
                "hostRequirements": {
                    "minimumVersion": env!("CARGO_PKG_VERSION"),
                    "capabilities": capabilities
                },
                "managedToolDependencies": [dependency]
            }))
            .unwrap()
        };
        let dependency = json!({
            "id": "baijimu-cli",
            "minimumVersion": "0.1.45",
            "requiredFor": ["install", "start"],
            "executablePathEnv": "CODEX_CONNECTOR_BAIJIMU_BINARY"
        });

        let valid = manifest(
            dependency.clone(),
            json!(["connector.managed-tool-dependencies.v1"]),
            json!({}),
        );
        validate_manifest(&valid).unwrap();
        assert_eq!(
            valid.managed_tool_dependencies[0].required_for,
            vec![
                ConnectorManagedToolDependencyPhase::Install,
                ConnectorManagedToolDependencyPhase::Start
            ]
        );

        let missing_capability = manifest(dependency.clone(), json!([]), json!({}));
        assert!(validate_manifest(&missing_capability)
            .unwrap_err()
            .to_string()
            .contains("connector.managed-tool-dependencies.v1"));

        let overridden = manifest(
            dependency,
            json!(["connector.managed-tool-dependencies.v1"]),
            json!({"CODEX_CONNECTOR_BAIJIMU_BINARY": "baijimu"}),
        );
        assert!(validate_manifest(&overridden)
            .unwrap_err()
            .to_string()
            .contains("cannot override"));
    }

    #[test]
    fn connector_manifest_validates_host_managed_process_contract() {
        let manifest = |stop_args: Value| {
            serde_json::from_value::<ConnectorManifest>(json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.host-managed",
                "name": "Host Managed Connector",
                "version": "1.0.0",
                "runtime": {
                    "type": "process",
                    "command": "host-managed-connector",
                    "args": ["run"],
                    "stopArgs": stop_args,
                    "processOwnership": "host"
                },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }],
                "hostRequirements": {
                    "minimumVersion": env!("CARGO_PKG_VERSION"),
                    "capabilities": ["connector.process.host-managed.v1"]
                }
            }))
            .unwrap()
        };

        let valid = manifest(json!(["stop"]));
        validate_manifest(&valid).unwrap();
        assert_eq!(
            valid.runtime.unwrap().process_ownership,
            ConnectorProcessOwnership::Host
        );

        let missing_stop = manifest(json!([]));
        assert!(validate_manifest(&missing_stop)
            .unwrap_err()
            .to_string()
            .contains("runtime.stopArgs"));
    }

    #[test]
    fn connector_manifest_validates_database_migration_contract() {
        let manifest = |database: Value| {
            serde_json::from_value::<ConnectorManifest>(json!({
                "schemaVersion": "3.0.0",
                "appId": "com.baijimu.connector.database-contract",
                "name": "Database Contract Connector",
                "version": "1.0.0",
                "runtime": { "type": "process", "command": "database-contract" },
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }],
                "upgradeReview": {
                    "configuration": "not_applicable",
                    "interfaces": "declared",
                    "database": "declared"
                },
                "database": database
            }))
            .unwrap()
        };
        let valid_database = json!({
            "engine": "sqlite",
            "schemaVersion": "2",
            "migrations": [{
                "id": "002-add-status",
                "fromVersion": "1",
                "toVersion": "2",
                "description": "Add message status",
                "changes": [{
                    "operation": "add_column",
                    "target": "messages.status",
                    "description": "Add status column"
                }],
                "rollback": "automatic",
                "downtime": "none"
            }]
        });

        validate_manifest(&manifest(valid_database.clone())).unwrap();

        let mut mismatched_review = manifest(valid_database.clone());
        mismatched_review.upgrade_review.as_mut().unwrap().database = "not_applicable".to_string();
        assert!(validate_manifest(&mismatched_review)
            .unwrap_err()
            .to_string()
            .contains("upgradeReview.database"));

        let mut missing_changes = valid_database.clone();
        missing_changes["migrations"][0]["changes"] = json!([]);
        assert!(validate_manifest(&manifest(missing_changes))
            .unwrap_err()
            .to_string()
            .contains("at least one change"));

        let mut unsupported_rollback_value = valid_database;
        unsupported_rollback_value["migrations"][0]["rollback"] = json!("sometimes");
        assert!(validate_manifest(&manifest(unsupported_rollback_value))
            .unwrap_err()
            .to_string()
            .contains("rollback must be"));
    }
