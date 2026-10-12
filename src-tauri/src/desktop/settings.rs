use super::*;

fn section_fields(section: &str) -> Result<&'static [&'static str], String> {
    match section {
        "identity" => Ok(&[
            "device.name",
            "device.description",
            "device.tags",
            "relay.agent_id",
        ]),
        "connection" => Ok(&[
            "platform.base_url",
            "relay.url",
            "relay.reconnect_secs",
            "upload.prepare_url",
            "upload.inline_limit_bytes",
            "upload.timeout_secs",
        ]),
        "events" => Ok(&["runtime.event_delivery"]),
        "runtime" => Ok(&[
            "runtime.python_path",
            "runtime.node_path",
            "runtime.default_timeout_secs",
            "runtime.max_timeout_secs",
            "runtime.log_limit",
            "runtime.log_file_enabled",
            "runtime.log_file_dir",
            "runtime.log_file_max_bytes",
            "runtime.log_file_max_files",
            "runtime.event_server_enabled",
            "runtime.event_server_bind",
        ]),
        _ => Err("未知设置分类".into()),
    }
}

fn apply_settings_patch(
    config: &AgentConfig,
    section: &str,
    changes: Value,
) -> Result<AgentConfig, String> {
    let allowed = section_fields(section)?;
    let changes = changes.as_object().ok_or("设置必须为对象")?;
    if changes.is_empty() || changes.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("设置包含当前分类不允许修改的字段".into());
    }
    let mut value = serde_json::to_value(config).map_err(|err| err.to_string())?;
    for (key, entry) in changes {
        let (group, field) = key.split_once('.').ok_or("无效设置字段")?;
        value[group][field] = entry.clone();
    }
    let next: AgentConfig = serde_json::from_value(value).map_err(|err| err.to_string())?;
    if !config.relay.token.is_empty()
        && bridge_agent::config::environment::api_base(&config.platform.base_url)
            != bridge_agent::config::environment::api_base(&next.platform.base_url)
    {
        return Err("切换环境请先完成浏览器授权，原连接尚未修改".into());
    }
    next.validate().map_err(|err| err.to_string())?;
    Ok(next)
}

#[tauri::command]
pub(super) async fn save_settings_section(
    state: tauri::State<'_, DesktopState>,
    section: String,
    changes: Value,
) -> Result<ConfigDocument, String> {
    // Read the latest document and patch only explicitly editable fields. In particular,
    // credentials, workspace identity, installed apps and other sections never come from the UI.
    let current = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let next = apply_settings_patch(&current, &section, changes)?;
    save_agent_config(&state.config_path, &next).map_err(|err| err.to_string())?;
    load_config(state).await
}

#[tauri::command]
pub(super) fn get_desktop_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|err| err.to_string())
}

fn change_autostart(
    enabled: bool,
    read: impl Fn() -> Result<bool, String>,
    write: impl FnOnce(bool) -> Result<(), String>,
) -> Result<bool, String> {
    if read()? != enabled {
        write(enabled)?;
    }
    let actual = read()?;
    if actual != enabled {
        return Err("系统自启动状态未按预期改变，请检查系统登录项权限后重试".into());
    }
    Ok(actual)
}

#[tauri::command]
pub(super) fn set_desktop_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let actual = change_autostart(
        enabled,
        || get_desktop_autostart(app.clone()),
        |enabled| {
            if enabled {
                app.autolaunch().enable()
            } else {
                app.autolaunch().disable()
            }
            .map_err(|err| err.to_string())
        },
    )?;
    app.state::<DesktopState>().startup_health.set_component(
        "autostart",
        "登录启动",
        "ready",
        Some(if actual { "已开启" } else { "已关闭" }.into()),
    );
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_changes_are_verified_and_idempotent() {
        let actual = std::cell::Cell::new(true);
        let writes = std::cell::Cell::new(0);
        for target in [false, false, true] {
            let result = change_autostart(
                target,
                || Ok(actual.get()),
                |value| {
                    writes.set(writes.get() + 1);
                    actual.set(value);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(result, target);
        }
        assert_eq!(writes.get(), 2);
    }

    #[test]
    fn autostart_does_not_claim_success_on_failed_or_ignored_writes() {
        assert!(change_autostart(false, || Ok(true), |_| Err("denied".into())).is_err());
        assert!(change_autostart(false, || Ok(true), |_| Ok(())).is_err());
        assert!(change_autostart(
            false,
            || Err("unreadable".into()),
            |_| panic!("must not write without reading state")
        )
        .is_err());
    }

    #[test]
    fn section_save_preserves_credentials_and_other_sections() {
        let mut config = AgentConfig::example();
        config.relay.token = "private-token".into();
        let before = serde_json::to_value(&config).unwrap();
        let next = apply_settings_patch(
            &config,
            "identity",
            serde_json::json!({"device.name": "新设备"}),
        )
        .unwrap();
        let after = serde_json::to_value(&next).unwrap();
        assert_eq!(next.device.name, "新设备");
        for key in [
            "platform",
            "relay",
            "runtime",
            "upload",
            "services",
            "local_apps",
        ] {
            assert_eq!(before[key], after[key], "{key}");
        }
    }

    #[test]
    fn section_save_rejects_cross_section_and_secret_fields() {
        let config = AgentConfig::example();
        for key in [
            "relay.token",
            "platform.workspace_id",
            "runtime.python_path",
            "services",
        ] {
            assert!(
                apply_settings_patch(&config, "identity", serde_json::json!({key: "bad"})).is_err()
            );
        }
        assert!(apply_settings_patch(&config, "unknown", serde_json::json!({})).is_err());
    }

    #[test]
    fn connection_save_preserves_authorization_and_rejects_environment_switch() {
        let mut config = AgentConfig::example();
        config.relay.token = "private-token".into();
        let next = apply_settings_patch(
            &config,
            "connection",
            serde_json::json!({"relay.reconnect_secs": 12}),
        )
        .unwrap();
        assert_eq!(next.relay.token, config.relay.token);
        assert_eq!(next.platform.workspace_id, config.platform.workspace_id);
        assert!(apply_settings_patch(
            &config,
            "connection",
            serde_json::json!({"platform.base_url": "https://different.example"})
        )
        .is_err());
        assert!(apply_settings_patch(
            &config,
            "runtime",
            serde_json::json!({"runtime.max_timeout_secs": -1})
        )
        .is_err());
    }

    #[test]
    fn runtime_save_preserves_event_policy() {
        let config = AgentConfig::example();
        let next = apply_settings_patch(
            &config,
            "runtime",
            serde_json::json!({"runtime.python_path": "/usr/bin/python3"}),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&config.runtime.event_delivery).unwrap(),
            serde_json::to_value(&next.runtime.event_delivery).unwrap()
        );
    }
}
