fn validate_local_app_capabilities(app: &LocalAppConfig) -> Result<()> {
    let mut method_names = BTreeSet::new();
    let mut event_names = BTreeSet::new();
    for method in &app.methods {
        if method.name.trim().is_empty() {
            bail!("method name cannot be empty in local app `{}`", app.app_id);
        }
        if !method_names.insert(method.name.as_str()) {
            bail!(
                "duplicate method `{}` in local app `{}`",
                method.name,
                app.app_id
            );
        }
        if let MethodBinding::Http(binding) = &method.binding {
            if binding.url.trim().is_empty() || binding.http_method.trim().is_empty() {
                bail!(
                    "http binding cannot be empty for local app {}.{}",
                    app.app_id,
                    method.name
                );
            }
        }
    }
    for event in &app.events {
        if event.name.trim().is_empty() {
            bail!("event name cannot be empty in local app `{}`", app.app_id);
        }
        if !event_names.insert(event.name.as_str()) {
            bail!(
                "duplicate event `{}` in local app `{}`",
                event.name,
                app.app_id
            );
        }
        if method_names.contains(event.name.as_str()) {
            bail!(
                "event `{}` conflicts with method of the same name in local app `{}`",
                event.name,
                app.app_id
            );
        }
    }
    Ok(())
}
