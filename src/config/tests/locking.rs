#[test]
fn update_config_keeps_concurrent_service_inserts() {
    use std::thread;
    use std::time::Duration;

    let dir = tempdir().unwrap();
    let path = dir.path().join("agent-config.json");
    save_config(&path, &AgentConfig::example()).unwrap();
    let mut handles = Vec::new();
    for name in ["alpha-service", "beta-service"] {
        let path = path.clone();
        handles.push(thread::spawn(move || {
            update_config(&path, |config| {
                thread::sleep(Duration::from_millis(40));
                let mut service = config.services[0].clone();
                service.name = name.to_string();
                config.services.push(service);
                Ok(())
            })
            .unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let loaded = load_config(&path).unwrap();
    let names = loaded
        .services
        .iter()
        .map(|service| service.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"alpha-service"), "{names:?}");
    assert!(names.contains(&"beta-service"), "{names:?}");
    assert!(path.with_file_name("agent-config.json.lock").is_file());
}

#[test]
fn overlay_submitted_config_preserves_disk_only_records_and_blank_secrets() {
    let mut current = AgentConfig::example();
    current.relay.token = "relay-secret".to_string();
    current.runtime.service_registration_token = Some("registration-secret".to_string());
    let mut disk_service = current.services[0].clone();
    disk_service.name = "registered-later".to_string();
    current.services.push(disk_service);
    let mut disk_app = LocalAppConfig {
        app_id: "com.example.added".to_string(),
        name: "Added".to_string(),
        version: "1.0.0".to_string(),
        description: String::new(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: Vec::new(),
        events: Vec::new(),
    };
    current.local_apps.push(disk_app.clone());

    let mut submitted = current.clone();
    submitted.relay.token.clear();
    submitted.runtime.service_registration_token = None;
    submitted.services.pop();
    submitted.local_apps.clear();
    submitted.device.name = "renamed device".to_string();
    let edited = submitted.services[0].name.clone();

    overlay_submitted_config(&mut current, submitted);

    assert_eq!(current.relay.token, "relay-secret");
    assert_eq!(
        current.runtime.service_registration_token.as_deref(),
        Some("registration-secret")
    );
    assert_eq!(current.device.name, "renamed device");
    assert!(current
        .services
        .iter()
        .any(|service| service.name == edited));
    assert!(current
        .services
        .iter()
        .any(|service| service.name == "registered-later"));
    assert_eq!(current.local_apps, vec![disk_app]);
}
