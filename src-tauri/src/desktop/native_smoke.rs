// Only compiled in development builds; never part of signed release executables.
use super::*;
use tauri::Listener;

pub(super) fn enabled() -> bool {
    std::env::var_os("BRIDGE_AGENT_FRONTEND_SMOKE").is_some()
}

pub(super) fn setup(
    app: &tauri::App,
    config: &Path,
    health: &StartupHealthManager,
) -> anyhow::Result<bool> {
    if !enabled() {
        return Ok(false);
    }
    anyhow::ensure!(
        app.config().identifier.ends_with(".frontend-smoke"),
        "smoke requires isolated application identity"
    );
    anyhow::ensure!(
        std::env::var_os("WS_BRIDGE_CONFIG").is_some(),
        "smoke requires isolated config"
    );
    ensure_config_exists(config)?;
    health.set_component("config_migration", "配置迁移", "ready", None);
    mark_business_startup_skipped(health, "原生界面测试不启动设备服务");
    let handle = app.handle().clone();
    app.listen("desktop-smoke-result", move |event| {
        let result: Value = serde_json::from_str(event.payload()).unwrap_or_default();
        eprintln!("desktop-smoke-result: {result}");
        handle.exit(if result["success"] == true { 0 } else { 1 });
    });
    Ok(true)
}

pub(super) fn probe<R: tauri::Runtime>(webview: &tauri::Webview<R>) {
    if enabled() && webview.label() == "main" {
        if let Err(error) = webview.eval(include_str!("native_smoke.js")) {
            eprintln!("native smoke probe failed: {error}");
            webview.app_handle().exit(1);
        }
    }
}
