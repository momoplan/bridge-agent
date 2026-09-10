fn summary_from_record(record: ConnectorInstallRecord) -> ConnectorSummary {
    let last_synced_at_epoch_ms = if record.last_synced_at_epoch_ms == 0 {
        record.installed_at_epoch_ms
    } else {
        record.last_synced_at_epoch_ms
    };
    let start_policy = record
        .manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.start_policy.clone())
        .unwrap_or_else(default_connector_start_policy);
    let process_ownership = record
        .manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.process_ownership)
        .unwrap_or_default();
    let registrations =
        load_connector_service_registrations(Path::new(&record.package_path), &record.manifest)
            .unwrap_or_default();
    let methods = registrations
        .iter()
        .flat_map(|registration| registration.methods.iter())
        .map(|method| ConnectorMethodContract {
            name: method.name.clone(),
            description: method.description.clone(),
            input_schema: method.input_schema.clone(),
            response_mode: match method.response_mode {
                ResponseMode::Cmodel => "cmodel",
                ResponseMode::Plain => "plain",
                ResponseMode::Passthrough => "passthrough",
            }
            .to_string(),
            path: method.path.trim().to_string(),
            http_method: method.http_method.trim().to_uppercase(),
        })
        .collect::<Vec<_>>();
    let events = registrations
        .iter()
        .flat_map(|registration| registration.local_app_events.iter())
        .map(|event| ConnectorEventContract {
            name: event.name.clone(),
            description: event.description.clone(),
            payload_schema: event.payload_schema.clone(),
        })
        .collect::<Vec<_>>();
    let method_names = methods.iter().map(|method| method.name.clone()).collect();
    let event_names = events.iter().map(|event| event.name.clone()).collect();
    let icon_data_url = record
        .manifest
        .icon
        .as_ref()
        .map(connector_icon_data_url)
        .transpose()
        .unwrap_or_default();
    ConnectorSummary {
        install_source: record.install_source,
        app_id: record.manifest.app_id,
        name: record.manifest.name,
        version: record.manifest.version,
        package_path: record.package_path,
        source_path: record.source_path,
        source_reference: record.source_reference,
        review_status: record.review_status,
        source_checksum: record.source_checksum,
        package_checksum: record.package_checksum,
        icon_data_url,
        ui: record.manifest.ui,
        permissions: record.manifest.permissions,
        start_policy,
        process_ownership,
        config_schema: record.manifest.config_schema,
        database: record.manifest.database,
        methods,
        events,
        method_names,
        event_names,
        installed_at_epoch_ms: record.installed_at_epoch_ms,
        last_synced_at_epoch_ms,
    }
}
