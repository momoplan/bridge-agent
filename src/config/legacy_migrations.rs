fn migrate_legacy_defaults(config: &mut AgentConfig) -> bool {
    let mut changed = false;
    changed |= ensure_default_platform_base_url(config);
    if config.relay.url.trim() == LEGACY_DEFAULT_RELAY_URL {
        config.relay.url = DEFAULT_RELAY_URL.to_string();
        changed = true;
    }
    if config.upload.inline_limit_bytes == LEGACY_INLINE_LIMIT_BYTES {
        config.upload.inline_limit_bytes = DEFAULT_INLINE_LIMIT_BYTES;
        changed = true;
    }
    if config.device.name.trim() == LEGACY_DEFAULT_DEVICE_NAME {
        let device_name = default_device_name();
        if device_name != LEGACY_DEFAULT_DEVICE_NAME {
            config.device.name = device_name;
            changed = true;
        }
    }
    changed |= remove_legacy_default_local_java_service(config);
    changed
}

fn ensure_default_platform_base_url(config: &mut AgentConfig) -> bool {
    let base_url = environment::api_base(&config.platform.base_url);
    if base_url == config.platform.base_url {
        return false;
    }
    config.platform.base_url = base_url;
    true
}

fn remove_legacy_default_local_java_service(config: &mut AgentConfig) -> bool {
    let initial_len = config.services.len();
    config
        .services
        .retain(|service| !is_legacy_default_local_java_service(service));
    config.services.len() != initial_len
}

fn is_legacy_default_local_java_service(service: &ServiceConfig) -> bool {
    if service.name != "local-java-service"
        || service.description != "Example business service backed by a local HTTP endpoint."
        || service.enabled
        || service.health_check.is_some()
        || service.start_command.is_some()
        || service.stop_command.is_some()
        || service.methods.len() != 1
    {
        return false;
    }

    let method = &service.methods[0];
    method.name == "invokeApi"
        && method.description == "Forward invocation arguments to a local HTTP service."
        && method.enabled
        && method.input_schema == default_object_schema()
        && matches!(
            &method.binding,
            MethodBinding::Http(HttpBinding {
                url,
                http_method,
                headers,
                timeout_secs
            }) if url == "http://127.0.0.1:8081/api/invoke"
                && http_method == "POST"
                && headers.is_empty()
                && *timeout_secs == Some(20)
        )
}
