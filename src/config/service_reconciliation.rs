fn ensure_default_computer_methods(config: &mut AgentConfig) -> bool {
    let default_service = default_computer_service();
    let default_names: BTreeSet<String> = default_service
        .methods
        .iter()
        .map(|method| method.name.clone())
        .collect();

    if let Some(service) = config
        .services
        .iter_mut()
        .find(|service| service.name == "computer")
    {
        let existing_names: BTreeSet<String> = service
            .methods
            .iter()
            .map(|method| method.name.clone())
            .collect();
        let mut changed = false;

        for method in default_service.methods {
            if !existing_names.contains(&method.name) {
                service.methods.push(method);
                changed = true;
            }
        }

        if service.description.trim().is_empty() {
            service.description = default_service.description;
            changed = true;
        }

        if !service
            .methods
            .iter()
            .any(|method| default_names.contains(&method.name))
        {
            service.enabled = true;
            changed = true;
        }

        return changed;
    }

    config.services.insert(0, default_service);
    true
}

fn ensure_default_shell_service(config: &mut AgentConfig) -> bool {
    if config
        .services
        .iter()
        .any(|service| service.name == "shell")
    {
        return ensure_shell_service_methods(config, "shell", default_shell_service());
    }

    let insert_index = config
        .services
        .iter()
        .position(|service| service.name == "computer")
        .map(|index| index + 1)
        .unwrap_or(0);
    config
        .services
        .insert(insert_index, default_shell_service());
    true
}

fn ensure_shell_service_methods(
    config: &mut AgentConfig,
    service_name: &str,
    default_service: ServiceConfig,
) -> bool {
    if let Some(service) = config
        .services
        .iter_mut()
        .find(|service| service.name == service_name)
    {
        let mut changed = false;

        if service.description.trim().is_empty() {
            service.description = default_service.description;
            changed = true;
        }

        let existing_names = service
            .methods
            .iter()
            .map(|method| method.name.clone())
            .collect::<BTreeSet<_>>();
        for default_method in default_service.methods {
            if !existing_names.contains(&default_method.name) {
                service.methods.push(default_method);
                changed = true;
            }
        }

        return changed;
    }

    false
}

fn ensure_service_registration_defaults(config: &mut AgentConfig) -> bool {
    if !config.runtime.service_registration_enabled {
        return false;
    }

    if let Ok(bind) = config.runtime.event_server_bind.parse::<SocketAddr>() {
        if !bind.ip().is_loopback() {
            config.runtime.service_registration_enabled = false;
            return true;
        }
    }

    if config
        .runtime
        .service_registration_token
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        config.runtime.service_registration_token = Some(generate_registration_token());
        return true;
    }

    false
}
