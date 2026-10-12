// Test-only IPC fixture. Production bundles never import this module.
export function installDesktopRuntime(options: { dnsError?: boolean; marketError?: boolean; installError?: boolean; configError?: boolean; crash?: boolean; authorized?: boolean; settingsSaveError?: boolean; autostartError?: boolean } = {}) {
  const calls: string[] = [];
  const callbacks = new Map<number, (value: unknown) => void>();
  let nextId = 0;
  const health = { revision: 1, safeMode: false, forcedSafeMode: false, consecutiveFailures: 0,
    frontendReady: false, startupLogPath: "test-startup.log",
    components: [{ id: "config_migration", label: "配置迁移", status: "ready" }] };
  const runtime = { revision: 1, status: "stopped", last_event_at: 0, relay_registered: false };
  const update = { currentVersion: "1.0.0", latestVersion: "1.0.1", updateAvailable: true,
    forceUpdateRequired: false, autoDownloadAvailable: true, currentTarget: "test" };
  const connector = { appId: "fixture-app", name: "测试应用", description: "启动测试", version: "1.0.0",
    reviewStatus: "PUBLISHED", methodNames: [], eventNames: [], methods: [], events: [], permissions: [] };
  const config = {
    platform: { base_url: "https://example.test", workspace_id: options.authorized === false ? null : 42 },
    relay: { url: "wss://example.test", token: "", agent_id: "fixture-device", reconnect_secs: 5 },
    device: { name: options.crash ? {} : "测试设备", description: "", tags: [] },
    upload: { inline_limit_bytes: 1024, timeout_secs: 30 }, runtime: { node_path: null, python_path: null, default_timeout_secs: 30, max_timeout_secs: 120, log_limit: 500,
      log_file_enabled: true, log_file_dir: null, log_file_max_bytes: 5242880, log_file_max_files: 5,
      event_server_enabled: true, event_server_bind: "127.0.0.1:18081", event_delivery: { automatic_cleanup: true,
        retention_days: 7, capacity_bytes: 104857600, diagnostic_retention_days: 7 } },
    credential_status: { relay_token_configured: options.authorized !== false },
    services: ["computer", "shell"].map((name) => ({ name, description: name, enabled: true, methods: [] })),
    local_apps: [{ appId: connector.appId, name: connector.name, version: connector.version,
      description: "测试", enabled: true, methods: [], events: [] }]
  };
  const fixture = { calls, options, config, autostart: true, settingsWrites: [] as unknown[] };
  Object.assign(window, {
    desktopFixture: fixture,
    __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    __TAURI_INTERNALS__: {
      transformCallback(callback: (value: unknown) => void) { callbacks.set(++nextId, callback); return nextId; },
      async invoke(command: string, args?: Record<string, any>) {
        calls.push(command);
        switch (command) {
          case "plugin:event|listen": return ++nextId;
          case "plugin:deep-link|get_current": return [];
          case "get_startup_health": return health;
          case "mark_frontend_ready": return { ...health, frontendReady: true };
          case "app_version": return { currentVersion: "1.0.0", currentTarget: "test" };
          case "check_app_update":
            if (options.dnsError) throw new Error("dns error: failed to lookup address information");
            return update;
          case "install_app_update":
            if (options.installError) throw new Error("signature verification failed (test)");
            return { status: "installed", version: "1.0.1" };
          case "event_storage_statistics": return { pending_events: 0, logical_bytes: 0, physical_bytes: 0, expiring_events: 0, expiring_bytes: 0, last_cleanup_expired: 0 };
          case "get_desktop_autostart": return fixture.autostart;
          case "set_desktop_autostart":
            if (options.autostartError) throw new Error("系统拒绝修改登录项");
            fixture.autostart = args!.enabled;
            return fixture.autostart;
          case "save_settings_section":
            if (options.settingsSaveError) throw new Error("保存失败（故障注入）");
            fixture.settingsWrites.push(args);
            for (const [path, value] of Object.entries(args!.changes)) {
              const [group, key] = path.split(".");
              (config as unknown as Record<string, Record<string, unknown>>)[group][key] = value;
            }
            return { config, config_path: "fixture.json", manifest_preview: "{}", runtime };
          case "load_config":
            if (options.configError) throw new Error("配置读取失败（故障注入）");
            return { config, config_path: "fixture.json", manifest_preview: "{}", runtime };
          case "list_connector_apps": return [connector];
          case "runtime_snapshot": return runtime;
          case "list_market_connector_apps":
            if (options.marketError) throw new Error("请求 localApp 市场失败: HTTP 404");
            return [];
          case "baijimu_cli_status": return { id: "fixture-cli", name: "测试 CLI", description: "托管工具",
            state: "ready", installedVersion: "1.0.0", bundledVersion: "1.0.0", canRollback: false };
          case "list_connector_app_install_tasks":
          case "registered_service_statuses":
          case "local_app_runtime_statuses":
          case "connector_lifecycle_snapshots":
          case "list_logs": return [];
          default: return null;
        }
      }
    }
  });
  return fixture;
}
