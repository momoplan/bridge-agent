import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";

type RuntimeStatus =
  | "stopped"
  | "starting"
  | "connecting"
  | "online"
  | "backoff"
  | "stopping";

interface RuntimeSnapshot {
  status: RuntimeStatus;
  config_path: string | null;
  agent_id: string | null;
  relay_url: string | null;
  last_error: string | null;
  last_event_at: number;
}

interface LogEntry {
  timestamp_ms: number;
  level: string;
  message: string;
}

interface RelayConfig {
  url: string;
  agent_id: string;
  token: string;
  reconnect_secs: number;
}

interface PlatformConfig {
  base_url: string;
  workspace_id: number | null;
}

interface DeviceConfig {
  name: string;
  description: string;
  tags: string[];
}

interface RuntimeConfig {
  default_timeout_secs: number;
  max_timeout_secs: number;
  log_limit: number;
}

interface ShellBinding {
  type: "shell_command";
  root_dir: string;
  allow_commands: string[];
  default_timeout_secs?: number | null;
  max_timeout_secs?: number | null;
}

interface HttpBinding {
  type: "http";
  url: string;
  http_method: string;
  headers: Record<string, string>;
  timeout_secs?: number | null;
}

type MethodBinding = ShellBinding | HttpBinding;

interface MethodConfig {
  name: string;
  description: string;
  enabled: boolean;
  input_schema: unknown;
  binding: MethodBinding;
}

interface ServiceConfig {
  name: string;
  description: string;
  enabled: boolean;
  methods: MethodConfig[];
}

interface AgentConfig {
  platform: PlatformConfig;
  relay: RelayConfig;
  device: DeviceConfig;
  runtime: RuntimeConfig;
  services: ServiceConfig[];
}

interface BrowserAuthStartResponse {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string;
  expiresIn: number;
  interval: number;
}

interface BrowserAuthPollResponse {
  status: "pending" | "authorized" | "denied" | "expired";
  message: string;
  config: AgentConfig | null;
}

interface ConfigDocument {
  config_path: string;
  manifest_preview: string;
  config: AgentConfig;
  runtime: RuntimeSnapshot;
}

interface UiShellBinding {
  type: "shell_command";
  root_dir: string;
  allow_commands_text: string;
  default_timeout_secs: string;
  max_timeout_secs: string;
}

interface UiHttpBinding {
  type: "http";
  url: string;
  http_method: string;
  headers_text: string;
  timeout_secs: string;
}

type UiMethodBinding = UiShellBinding | UiHttpBinding;

interface UiMethodConfig {
  name: string;
  description: string;
  enabled: boolean;
  input_schema_text: string;
  binding: UiMethodBinding;
}

interface UiServiceConfig {
  name: string;
  description: string;
  enabled: boolean;
  methods: UiMethodConfig[];
}

interface UiAgentConfig {
  platform: {
    base_url: string;
    workspace_id: string;
  };
  relay: RelayConfig;
  device: {
    name: string;
    description: string;
    tags_text: string;
  };
  runtime: RuntimeConfig;
  services: UiServiceConfig[];
}

type SettingsSection = "identity" | "connection" | "runtime";
type DetailPanel = "status" | "logs" | "manifest";

const SHELL_SCHEMA = {
  type: "object",
  required: ["command"],
  properties: {
    command: {
      type: "array",
      items: { type: "string" },
      minItems: 1
    },
    cwd: { type: "string" },
    env: {
      type: "object",
      additionalProperties: { type: "string" }
    }
  }
};

const HTTP_SCHEMA = {
  type: "object",
  additionalProperties: true
};

const DEFAULT_PLATFORM_BASE_URL = "https://baijimu.com/lowcode3";
const DEFAULT_SAFE_COMMANDS = "echo, pwd, ls";
const FULL_ACCESS_COMMAND = "*";
const FULL_ACCESS_ROOT_DIR = "/";

function App() {
  const [configPath, setConfigPath] = useState("");
  const [manifestPreview, setManifestPreview] = useState("");
  const [config, setConfig] = useState<UiAgentConfig | null>(null);
  const [runtime, setRuntime] = useState<RuntimeSnapshot | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [browserAuth, setBrowserAuth] = useState<BrowserAuthStartResponse | null>(null);
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [activeSettingsSection, setActiveSettingsSection] =
    useState<SettingsSection>("identity");
  const [activeDetailPanel, setActiveDetailPanel] = useState<DetailPanel>("status");
  const [expandedServiceIndex, setExpandedServiceIndex] = useState<number | null>(0);

  useEffect(() => {
    void refreshAll();
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void refreshRuntime();
    }, 1500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!config) {
      return;
    }
    if (normalizePlatformBaseUrl(config.platform.base_url) !== DEFAULT_PLATFORM_BASE_URL) {
      setShowAdvancedSettings(true);
    }
  }, [config]);

  useEffect(() => {
    if (!config) {
      return;
    }
    setExpandedServiceIndex((current) => {
      if (config.services.length === 0) {
        return null;
      }
      if (current == null || current >= config.services.length) {
        return 0;
      }
      return current;
    });
  }, [config?.services.length]);

  const statusLabel = useMemo(() => {
    if (!runtime) {
      return "未加载";
    }
    const textMap: Record<RuntimeStatus, string> = {
      stopped: "已停止",
      starting: "启动中",
      connecting: "连接中",
      online: "在线",
      backoff: "重连等待",
      stopping: "停止中"
    };
    return textMap[runtime.status];
  }, [runtime]);

  const latestLog = logs.length > 0 ? logs[logs.length - 1] : null;
  const enabledServiceCount = config?.services.filter((service) => service.enabled).length ?? 0;

  async function refreshAll() {
    try {
      setError("");
      const document = await invoke<ConfigDocument>("load_config");
      setConfigPath(document.config_path);
      setManifestPreview(document.manifest_preview);
      setConfig(toUiConfig(document.config));
      setRuntime(document.runtime);
      const latestLogs = await invoke<LogEntry[]>("list_logs", { limit: 200 });
      setLogs(latestLogs);
    } catch (err) {
      setError(readError(err));
    }
  }

  async function refreshRuntime() {
    try {
      const [snapshot, latestLogs] = await Promise.all([
        invoke<RuntimeSnapshot>("runtime_snapshot"),
        invoke<LogEntry[]>("list_logs", { limit: 200 })
      ]);
      setRuntime(snapshot);
      setLogs(latestLogs);
    } catch (err) {
      setError(readError(err));
    }
  }

  async function saveOnly() {
    if (!config) {
      return;
    }
    try {
      setBusy(true);
      setMessage("");
      setError("");
      const document = await invoke<ConfigDocument>("save_config", {
        config: fromUiConfig(config)
      });
      setConfigPath(document.config_path);
      setManifestPreview(document.manifest_preview);
      setRuntime(document.runtime);
      setMessage("配置已保存");
    } catch (err) {
      setError(readError(err));
    } finally {
      setBusy(false);
    }
  }

  async function startAgent() {
    if (!config) {
      return;
    }
    try {
      setBusy(true);
      setMessage("");
      setError("");
      const snapshot = await invoke<RuntimeSnapshot>("start_agent", {
        config: fromUiConfig(config)
      });
      setRuntime(snapshot);
      setMessage("Agent 已启动");
      await refreshRuntime();
    } catch (err) {
      setError(readError(err));
    } finally {
      setBusy(false);
    }
  }

  async function stopAgent() {
    try {
      setBusy(true);
      setMessage("");
      setError("");
      const snapshot = await invoke<RuntimeSnapshot>("stop_agent");
      setRuntime(snapshot);
      setMessage("Agent 已停止");
      await refreshRuntime();
    } catch (err) {
      setError(readError(err));
    } finally {
      setBusy(false);
    }
  }

  async function resetExampleConfig() {
    try {
      setBusy(true);
      setMessage("");
      setError("");
      const document = await invoke<ConfigDocument>("reset_example_config");
      setConfigPath(document.config_path);
      setManifestPreview(document.manifest_preview);
      setConfig(toUiConfig(document.config));
      setRuntime(document.runtime);
      setMessage("已恢复示例配置");
    } catch (err) {
      setError(readError(err));
    } finally {
      setBusy(false);
    }
  }

  async function clearLogs() {
    try {
      await invoke("clear_logs");
      setLogs([]);
    } catch (err) {
      setError(readError(err));
    }
  }

  async function beginBrowserAuth() {
    if (!config) {
      return;
    }
    try {
      setBusy(true);
      setMessage("");
      setError("");
      const session = await invoke<BrowserAuthStartResponse>("start_browser_auth", {
        config: fromUiConfig(config)
      });
      setBrowserAuth(session);
      setMessage(`已打开浏览器授权页，用户码 ${session.userCode}。请在网页里选择工作区并完成批准。`);
    } catch (err) {
      setError(readError(err));
    } finally {
      setBusy(false);
    }
  }

  async function pollBrowserAuthSession() {
    if (!config || !browserAuth) {
      return;
    }
    try {
      const result = await invoke<BrowserAuthPollResponse>("poll_browser_auth", {
        config: fromUiConfig(config),
        deviceCode: browserAuth.deviceCode
      });
      if (result.status === "authorized" && result.config) {
        setConfig(toUiConfig(result.config));
        setBrowserAuth(null);
        setMessage("浏览器授权成功，relay token 已自动写回配置");
        return;
      }
      if (result.status === "denied" || result.status === "expired") {
        setBrowserAuth(null);
        setError(result.message);
      }
    } catch (err) {
      setBrowserAuth(null);
      setError(readError(err));
    }
  }

  function updateRelay<K extends keyof RelayConfig>(key: K, value: RelayConfig[K]) {
    setConfig((current) =>
      current
        ? {
            ...current,
            relay: {
              ...current.relay,
              [key]: value
            }
          }
        : current
    );
  }

  function updatePlatform<K extends "base_url" | "workspace_id">(
    key: K,
    value: UiAgentConfig["platform"][K]
  ) {
    setConfig((current) =>
      current
        ? {
            ...current,
            platform: {
              ...current.platform,
              [key]: value
            }
          }
        : current
    );
  }

  function updateDevice<K extends "name" | "description" | "tags_text">(
    key: K,
    value: UiAgentConfig["device"][K]
  ) {
    setConfig((current) =>
      current
        ? {
            ...current,
            device: {
              ...current.device,
              [key]: value
            }
          }
        : current
    );
  }

  function updateRuntime<K extends keyof RuntimeConfig>(key: K, value: RuntimeConfig[K]) {
    setConfig((current) =>
      current
        ? {
            ...current,
            runtime: {
              ...current.runtime,
              [key]: value
            }
          }
        : current
    );
  }

  function updateService(
    serviceIndex: number,
    updater: (service: UiServiceConfig) => UiServiceConfig
  ) {
    setConfig((current) => {
      if (!current) {
        return current;
      }
      const services = current.services.map((service, index) =>
        index === serviceIndex ? updater(service) : service
      );
      return { ...current, services };
    });
  }

  function addService() {
    const nextIndex = config?.services.length ?? 0;
    setExpandedServiceIndex(nextIndex);
    setConfig((current) =>
      current
        ? {
            ...current,
            services: [
              ...current.services,
              {
                name: "new-service",
                description: "Describe this business service.",
                enabled: true,
                methods: [createShellMethod()]
              }
            ]
          }
        : current
    );
  }

  function removeService(serviceIndex: number) {
    setExpandedServiceIndex((current) => {
      if (current == null) {
        return current;
      }
      if (current === serviceIndex) {
        return null;
      }
      if (current > serviceIndex) {
        return current - 1;
      }
      return current;
    });
    setConfig((current) =>
      current
        ? {
            ...current,
            services: current.services.filter((_, index) => index !== serviceIndex)
          }
        : current
    );
  }

  function addMethod(serviceIndex: number, type: "shell_command" | "http") {
    updateService(serviceIndex, (service) => ({
      ...service,
      methods: [
        ...service.methods,
        type === "shell_command" ? createShellMethod() : createHttpMethod()
      ]
    }));
  }

  function removeMethod(serviceIndex: number, methodIndex: number) {
    updateService(serviceIndex, (service) => ({
      ...service,
      methods: service.methods.filter((_, index) => index !== methodIndex)
    }));
  }

  function updateMethod(
    serviceIndex: number,
    methodIndex: number,
    updater: (method: UiMethodConfig) => UiMethodConfig
  ) {
    updateService(serviceIndex, (service) => ({
      ...service,
      methods: service.methods.map((method, index) =>
        index === methodIndex ? updater(method) : method
      )
    }));
  }

  function toggleServiceExpanded(serviceIndex: number) {
    setExpandedServiceIndex((current) => (current === serviceIndex ? null : serviceIndex));
  }

  function grantFullShellAccess(serviceIndex: number, methodIndex: number) {
    updateMethod(serviceIndex, methodIndex, (current) => {
      if (current.binding.type !== "shell_command") {
        return current;
      }
      return {
        ...current,
        binding: {
          ...current.binding,
          root_dir: FULL_ACCESS_ROOT_DIR,
          allow_commands_text: FULL_ACCESS_COMMAND
        }
      };
    });
  }

  function restoreSafeShellAccess(serviceIndex: number, methodIndex: number) {
    updateMethod(serviceIndex, methodIndex, (current) => {
      if (current.binding.type !== "shell_command") {
        return current;
      }
      return {
        ...current,
        binding: {
          ...current.binding,
          root_dir: ".",
          allow_commands_text: DEFAULT_SAFE_COMMANDS
        }
      };
    });
  }

  function renderSettingsSection() {
    if (!config) {
      return <div />;
    }

    switch (activeSettingsSection) {
      case "identity":
        return (
          <div className="form-grid">
            <Field label="设备名称" hint="展示给平台和授权页，用来识别这台机器。">
              <input
                value={config.device.name}
                onChange={(event) => updateDevice("name", event.target.value)}
              />
            </Field>
            <Field label="运行名称" hint="当前 agent 在 relay 侧使用的唯一标识。">
              <input
                value={config.relay.agent_id}
                onChange={(event) => updateRelay("agent_id", event.target.value)}
              />
            </Field>
            <Field label="设备标签">
              <input
                value={config.device.tags_text}
                onChange={(event) => updateDevice("tags_text", event.target.value)}
                placeholder="desktop, local"
              />
            </Field>
            <Field label="配置文件" hint="shell 的相对目录也会相对这份配置所在目录解析。">
              <input value={configPath} readOnly />
            </Field>
            <Field label="设备描述" wide>
              <textarea
                rows={3}
                value={config.device.description}
                onChange={(event) => updateDevice("description", event.target.value)}
              />
            </Field>
          </div>
        );
      case "connection":
        return (
          <>
            <div className="section-inline-actions">
              <button
                className="secondary"
                onClick={() => setShowAdvancedSettings((current) => !current)}
              >
                {showAdvancedSettings ? "收起高级设置" : "展开高级设置"}
              </button>
            </div>
            <div className="form-grid">
              <Field
                label="默认平台"
                hint="桌面端默认连接百积木生产环境，无需手工填写地址。"
              >
                <input value={DEFAULT_PLATFORM_BASE_URL} readOnly />
              </Field>
              <Field
                label="授权后工作区"
                hint="浏览器授权页批准时选择工作区，成功后会自动写回。"
              >
                <input
                  value={config.platform.workspace_id || ""}
                  readOnly
                  placeholder="在浏览器授权页里选择后自动写回"
                />
              </Field>
              {showAdvancedSettings ? (
                <Field
                  label="Baijimu Base URL"
                  hint="仅在测试环境或私有部署时修改。"
                  wide
                >
                  <input
                    value={config.platform.base_url}
                    onChange={(event) => updatePlatform("base_url", event.target.value)}
                    placeholder={DEFAULT_PLATFORM_BASE_URL}
                  />
                </Field>
              ) : null}
              <Field label="Relay WebSocket URL">
                <input
                  value={config.relay.url}
                  onChange={(event) => updateRelay("url", event.target.value)}
                />
              </Field>
              <Field label="Agent Token">
                <input
                  type="password"
                  value={config.relay.token}
                  onChange={(event) => updateRelay("token", event.target.value)}
                />
              </Field>
              <Field label="重连秒数">
                <input
                  type="number"
                  min={1}
                  value={config.relay.reconnect_secs}
                  onChange={(event) =>
                    updateRelay("reconnect_secs", safeNumber(event.target.value, 3))
                  }
                />
              </Field>
            </div>
          </>
        );
      case "runtime":
        return (
          <div className="form-grid">
            <Field label="默认超时秒数">
              <input
                type="number"
                min={1}
                value={config.runtime.default_timeout_secs}
                onChange={(event) =>
                  updateRuntime("default_timeout_secs", safeNumber(event.target.value, 30))
                }
              />
            </Field>
            <Field label="最大超时秒数">
              <input
                type="number"
                min={1}
                value={config.runtime.max_timeout_secs}
                onChange={(event) =>
                  updateRuntime("max_timeout_secs", safeNumber(event.target.value, 120))
                }
              />
            </Field>
            <Field label="日志上限" hint="仅保留本地日志，不会上报到 relay。">
              <input
                type="number"
                min={50}
                value={config.runtime.log_limit}
                onChange={(event) =>
                  updateRuntime("log_limit", safeNumber(event.target.value, 500))
                }
              />
            </Field>
          </div>
        );
    }
  }

  function renderDetailPanel() {
    if (!config) {
      return <div />;
    }

    if (activeDetailPanel === "logs") {
      return (
        <Card
          title="运行日志"
          description="仅保留本地最近日志，可用于排查连接和调用问题。"
          action={
            <button className="ghost" onClick={() => void clearLogs()}>
              清空日志
            </button>
          }
        >
          <div className="log-panel">
            {logs.length === 0 ? (
              <div className="empty-state">暂无日志</div>
            ) : (
              logs.map((entry, index) => (
                <div className={`log-line log-${entry.level}`} key={`${entry.timestamp_ms}-${index}`}>
                  <span>{formatTime(entry.timestamp_ms)}</span>
                  <strong>{entry.level.toUpperCase()}</strong>
                  <p>{entry.message}</p>
                </div>
              ))
            )}
          </div>
        </Card>
      );
    }

    if (activeDetailPanel === "manifest") {
      return (
        <Card title="服务清单预览" description="这里展示将对外暴露的 manifest 形态。">
          <pre className="code-panel">{manifestPreview}</pre>
        </Card>
      );
    }

    return (
      <Card title="运行状态" description="查看当前连接快照、本地配置位置和最近错误。">
        <div className="status-detail-grid">
          <InfoRow label="当前状态" value={statusLabel} />
          <InfoRow label="最近事件" value={runtime ? formatTime(runtime.last_event_at) : "-"} />
          <InfoRow label="运行名称" value={runtime?.agent_id ?? config.relay.agent_id} />
          <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
          <InfoRow label="配置文件" value={configPath} />
          <InfoRow
            label="最近错误"
            value={runtime?.last_error || "无"}
            tone={runtime?.last_error ? "danger" : "normal"}
          />
        </div>
      </Card>
    );
  }

  useEffect(() => {
    if (!browserAuth) {
      return;
    }
    const timer = window.setInterval(() => {
      void pollBrowserAuthSession();
    }, Math.max(browserAuth.interval, 3) * 1000);
    return () => window.clearInterval(timer);
  }, [browserAuth, config]);

  if (!config) {
    return (
      <main className="app-shell">
        <section className="hero loading-panel">
          <div className="hero-copy">
            <p className="eyebrow">Bridge Agent</p>
            <h1>正在加载本地管理台</h1>
            <p>读取默认配置、运行状态和日志中。</p>
            {error ? <div className="alert error">{error}</div> : null}
          </div>
        </section>
      </main>
    );
  }

  return (
    <main className="app-shell">
      <section className="hero">
        <div className="hero-copy">
          <p className="eyebrow">Bridge Agent Desktop</p>
          <h1>本地服务暴露与运行控制台</h1>
          <p>
            这里管理这台机器要暴露出去的业务服务，也控制 shell 和本地 HTTP 绑定。
          </p>
        </div>
        <div className="hero-panel">
          <div className="hero-topline">
            <div className={`status-pill status-${runtime?.status ?? "stopped"}`}>{statusLabel}</div>
            <div className="hero-summary">
              <span>已启用服务 {enabledServiceCount}</span>
              <span>{latestLog ? `最近日志 ${formatTime(latestLog.timestamp_ms)}` : "暂无日志"}</span>
            </div>
          </div>
          <div className="hero-metrics">
            <div>
              <span>运行名称</span>
              <strong>{runtime?.agent_id ?? config.relay.agent_id}</strong>
            </div>
            <div>
              <span>Relay</span>
              <strong>{runtime?.relay_url ?? config.relay.url}</strong>
            </div>
            <div>
              <span>设备名称</span>
              <strong>{config.device.name}</strong>
            </div>
          </div>
          <div className="hero-actions">
            <button className="primary" onClick={() => void saveOnly()} disabled={busy}>
              保存配置
            </button>
            <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
              浏览器授权
            </button>
            <button className="primary accent" onClick={() => void startAgent()} disabled={busy}>
              启动 Agent
            </button>
            <button className="secondary" onClick={() => void stopAgent()} disabled={busy}>
              停止 Agent
            </button>
            <button className="ghost" onClick={() => void resetExampleConfig()} disabled={busy}>
              恢复示例
            </button>
          </div>
          <div className="detail-indicators">
            <button
              className={`detail-indicator ${activeDetailPanel === "status" ? "active" : ""}`}
              onClick={() => setActiveDetailPanel("status")}
            >
              <span>运行状态</span>
              <strong>{statusLabel}</strong>
              <small>{runtime?.last_error ? "有错误详情" : "点击查看连接详情"}</small>
            </button>
            <button
              className={`detail-indicator ${activeDetailPanel === "logs" ? "active" : ""}`}
              onClick={() => setActiveDetailPanel("logs")}
            >
              <span>运行日志</span>
              <strong>{logs.length} 条</strong>
              <small>{latestLog ? latestLog.message : "暂无日志"}</small>
            </button>
            <button
              className={`detail-indicator ${activeDetailPanel === "manifest" ? "active" : ""}`}
              onClick={() => setActiveDetailPanel("manifest")}
            >
              <span>服务清单</span>
              <strong>{config.services.length} 个服务</strong>
              <small>点击查看对外暴露预览</small>
            </button>
          </div>
          {message ? <div className="alert success">{message}</div> : null}
          {error ? <div className="alert error">{error}</div> : null}
          {runtime?.last_error ? <div className="alert warning">{runtime.last_error}</div> : null}
          {browserAuth ? (
            <div className="alert warning">
              等待浏览器授权中，用户码 {browserAuth.userCode}。请在浏览器中选择工作区。
            </div>
          ) : null}
        </div>
      </section>

      <section className="workspace-grid">
        <div className="column-main">
          <Card title="设置中心" description="设备信息、连接参数和运行策略都统一收在这里。">
            <div className="section-tabs">
              <button
                className={`section-tab ${activeSettingsSection === "identity" ? "active" : ""}`}
                onClick={() => setActiveSettingsSection("identity")}
              >
                基础信息
              </button>
              <button
                className={`section-tab ${activeSettingsSection === "connection" ? "active" : ""}`}
                onClick={() => setActiveSettingsSection("connection")}
              >
                连接配置
              </button>
              <button
                className={`section-tab ${activeSettingsSection === "runtime" ? "active" : ""}`}
                onClick={() => setActiveSettingsSection("runtime")}
              >
                运行策略
              </button>
            </div>
            {renderSettingsSection()}
          </Card>

          <Card
            title="业务服务"
            description="先看服务列表，点开单个服务后再编辑方法和具体绑定。"
            action={
              <button className="secondary" onClick={addService}>
                新增服务
              </button>
            }
          >
            <div className="service-list">
              {config.services.map((service, serviceIndex) => (
                <div
                  className={`service-card ${expandedServiceIndex === serviceIndex ? "expanded" : ""}`}
                  key={`${service.name}-${serviceIndex}`}
                >
                  <div className="service-summary-row">
                    <button
                      className="service-summary-button"
                      onClick={() => toggleServiceExpanded(serviceIndex)}
                    >
                      <div className="service-summary-copy">
                        <div className="service-title-row">
                          <h3>{service.name || "未命名服务"}</h3>
                          <span className={`service-badge ${service.enabled ? "enabled" : "disabled"}`}>
                            {service.enabled ? "已启用" : "已停用"}
                          </span>
                        </div>
                        <p>{service.description || "填写服务说明。"}</p>
                      </div>
                      <div className="service-summary-meta">
                        <span>{service.methods.length} 个方法</span>
                        <strong>{expandedServiceIndex === serviceIndex ? "收起详情" : "展开详情"}</strong>
                      </div>
                    </button>
                    <div className="service-actions">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={service.enabled}
                          onChange={(event) =>
                            updateService(serviceIndex, (current) => ({
                              ...current,
                              enabled: event.target.checked
                            }))
                          }
                        />
                        启用
                      </label>
                      <button className="ghost danger" onClick={() => removeService(serviceIndex)}>
                        删除服务
                      </button>
                    </div>
                  </div>
                  {expandedServiceIndex === serviceIndex ? (
                    <div className="service-details">
                      <div className="form-grid">
                        <Field label="服务名">
                          <input
                            value={service.name}
                            onChange={(event) =>
                              updateService(serviceIndex, (current) => ({
                                ...current,
                                name: event.target.value
                              }))
                            }
                          />
                        </Field>
                        <Field label="服务描述">
                          <input
                            value={service.description}
                            onChange={(event) =>
                              updateService(serviceIndex, (current) => ({
                                ...current,
                                description: event.target.value
                              }))
                            }
                          />
                        </Field>
                      </div>
                      <div className="method-toolbar">
                        <button
                          className="secondary"
                          onClick={() => addMethod(serviceIndex, "shell_command")}
                        >
                          新增 Shell 方法
                        </button>
                        <button
                          className="secondary"
                          onClick={() => addMethod(serviceIndex, "http")}
                        >
                          新增 HTTP 方法
                        </button>
                      </div>
                      <div className="method-list">
                        {service.methods.map((method, methodIndex) => (
                          <div
                            className="method-card"
                            key={`${service.name}-${method.name}-${methodIndex}`}
                          >
                            <div className="service-head">
                              <div>
                                <h4>{method.name || "未命名方法"}</h4>
                                <p>{method.description || "填写方法说明。"}</p>
                              </div>
                              <div className="service-actions">
                                <label className="switch">
                                  <input
                                    type="checkbox"
                                    checked={method.enabled}
                                    onChange={(event) =>
                                      updateMethod(serviceIndex, methodIndex, (current) => ({
                                        ...current,
                                        enabled: event.target.checked
                                      }))
                                    }
                                  />
                                  启用
                                </label>
                                <button
                                  className="ghost danger"
                                  onClick={() => removeMethod(serviceIndex, methodIndex)}
                                >
                                  删除方法
                                </button>
                              </div>
                            </div>
                            <div className="form-grid">
                              <Field label="方法名">
                                <input
                                  value={method.name}
                                  onChange={(event) =>
                                    updateMethod(serviceIndex, methodIndex, (current) => ({
                                      ...current,
                                      name: event.target.value
                                    }))
                                  }
                                />
                              </Field>
                              <Field label="方法描述">
                                <input
                                  value={method.description}
                                  onChange={(event) =>
                                    updateMethod(serviceIndex, methodIndex, (current) => ({
                                      ...current,
                                      description: event.target.value
                                    }))
                                  }
                                />
                              </Field>
                              <Field label="本地绑定类型">
                                <select
                                  value={method.binding.type}
                                  onChange={(event) =>
                                    updateMethod(serviceIndex, methodIndex, (current) => ({
                                      ...current,
                                      input_schema_text:
                                        event.target.value === "shell_command"
                                          ? prettyJson(SHELL_SCHEMA)
                                          : prettyJson(HTTP_SCHEMA),
                                      binding:
                                        event.target.value === "shell_command"
                                          ? createShellMethod().binding
                                          : createHttpMethod().binding
                                    }))
                                  }
                                >
                                  <option value="shell_command">shell_command</option>
                                  <option value="http">http</option>
                                </select>
                              </Field>
                            </div>

                            {method.binding.type === "shell_command" ? (
                              <>
                                <div className="permission-banner">
                                  <div>
                                    <strong>
                                      {isFullShellAccess(method.binding)
                                        ? "当前是全部权限模式"
                                        : "当前是受限模式"}
                                    </strong>
                                    <p>
                                      全部权限会把根目录设为 <code>/</code>，允许命令设为
                                      <code>*</code>。受限模式默认只开放少量命令。
                                    </p>
                                  </div>
                                  <div className="service-actions">
                                    {isFullShellAccess(method.binding) ? (
                                      <button
                                        className="ghost"
                                        onClick={() =>
                                          restoreSafeShellAccess(serviceIndex, methodIndex)
                                        }
                                      >
                                        恢复受限模式
                                      </button>
                                    ) : (
                                      <button
                                        className="secondary"
                                        onClick={() => grantFullShellAccess(serviceIndex, methodIndex)}
                                      >
                                        授予全部权限
                                      </button>
                                    )}
                                  </div>
                                </div>
                                <div className="form-grid">
                                  <Field
                                    label="根目录"
                                    hint="默认是配置目录；填 / 表示整个文件系统根目录。"
                                  >
                                    <input
                                      value={method.binding.root_dir}
                                      onChange={(event) =>
                                        updateMethod(serviceIndex, methodIndex, (current) => ({
                                          ...current,
                                          binding: {
                                            ...current.binding,
                                            root_dir: event.target.value
                                          }
                                        }))
                                      }
                                    />
                                  </Field>
                                  <Field
                                    label="允许命令"
                                    hint="逗号分隔；填 * 表示允许任意命令。"
                                  >
                                    <input
                                      value={method.binding.allow_commands_text}
                                      onChange={(event) =>
                                        updateMethod(serviceIndex, methodIndex, (current) => ({
                                          ...current,
                                          binding: {
                                            ...current.binding,
                                            allow_commands_text: event.target.value
                                          }
                                        }))
                                      }
                                      placeholder="echo, pwd, git 或 *"
                                    />
                                  </Field>
                                  <Field label="默认超时">
                                    <input
                                      value={method.binding.default_timeout_secs}
                                      onChange={(event) =>
                                        updateMethod(serviceIndex, methodIndex, (current) => ({
                                          ...current,
                                          binding: {
                                            ...current.binding,
                                            default_timeout_secs: event.target.value
                                          }
                                        }))
                                      }
                                      placeholder="留空则使用全局"
                                    />
                                  </Field>
                                  <Field label="最大超时">
                                    <input
                                      value={method.binding.max_timeout_secs}
                                      onChange={(event) =>
                                        updateMethod(serviceIndex, methodIndex, (current) => ({
                                          ...current,
                                          binding: {
                                            ...current.binding,
                                            max_timeout_secs: event.target.value
                                          }
                                        }))
                                      }
                                      placeholder="留空则使用全局"
                                    />
                                  </Field>
                                </div>
                              </>
                            ) : (
                              <div className="form-grid">
                                <Field label="本地 URL" wide>
                                  <input
                                    value={method.binding.url}
                                    onChange={(event) =>
                                      updateMethod(serviceIndex, methodIndex, (current) => ({
                                        ...current,
                                        binding: {
                                          ...current.binding,
                                          url: event.target.value
                                        }
                                      }))
                                    }
                                  />
                                </Field>
                                <Field label="HTTP 方法">
                                  <input
                                    value={method.binding.http_method}
                                    onChange={(event) =>
                                      updateMethod(serviceIndex, methodIndex, (current) => ({
                                        ...current,
                                        binding: {
                                          ...current.binding,
                                          http_method: event.target.value.toUpperCase()
                                        }
                                      }))
                                    }
                                  />
                                </Field>
                                <Field label="超时">
                                  <input
                                    value={method.binding.timeout_secs}
                                    onChange={(event) =>
                                      updateMethod(serviceIndex, methodIndex, (current) => ({
                                        ...current,
                                        binding: {
                                          ...current.binding,
                                          timeout_secs: event.target.value
                                        }
                                      }))
                                    }
                                    placeholder="留空则使用全局"
                                  />
                                </Field>
                                <Field label="请求头" wide>
                                  <textarea
                                    rows={4}
                                    value={method.binding.headers_text}
                                    onChange={(event) =>
                                      updateMethod(serviceIndex, methodIndex, (current) => ({
                                        ...current,
                                        binding: {
                                          ...current.binding,
                                          headers_text: event.target.value
                                        }
                                      }))
                                    }
                                    placeholder={"Authorization: Bearer xxx\nX-App: local-java"}
                                  />
                                </Field>
                              </div>
                            )}

                            <Field label="输入 Schema JSON" wide>
                              <textarea
                                rows={8}
                                value={method.input_schema_text}
                                onChange={(event) =>
                                  updateMethod(serviceIndex, methodIndex, (current) => ({
                                    ...current,
                                    input_schema_text: event.target.value
                                  }))
                                }
                              />
                            </Field>
                          </div>
                        ))}
                      </div>
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          </Card>
        </div>

        <div className="column-side">{renderDetailPanel()}</div>
      </section>
    </main>
  );
}

function Field(props: {
  label: string;
  children: JSX.Element;
  wide?: boolean;
  hint?: string;
}) {
  return (
    <label className={`field ${props.wide ? "field-wide" : ""}`}>
      <span>{props.label}</span>
      {props.children}
      {props.hint ? <small className="field-hint">{props.hint}</small> : null}
    </label>
  );
}

function Card(props: {
  title: string;
  description: string;
  children: JSX.Element | JSX.Element[];
  action?: JSX.Element;
}) {
  return (
    <section className="card">
      <div className="card-head">
        <div>
          <h2>{props.title}</h2>
          <p>{props.description}</p>
        </div>
        {props.action ?? null}
      </div>
      {props.children}
    </section>
  );
}

function InfoRow(props: {
  label: string;
  value: string;
  tone?: "normal" | "danger";
}) {
  return (
    <div className="info-row">
      <span>{props.label}</span>
      <strong className={props.tone === "danger" ? "danger-text" : ""}>{props.value}</strong>
    </div>
  );
}

function toUiConfig(config: AgentConfig): UiAgentConfig {
  return {
    platform: {
      base_url: normalizePlatformBaseUrl(config.platform.base_url),
      workspace_id:
        config.platform.workspace_id == null ? "" : String(config.platform.workspace_id)
    },
    relay: config.relay,
    device: {
      name: config.device.name,
      description: config.device.description,
      tags_text: config.device.tags.join(", ")
    },
    runtime: config.runtime,
    services: config.services.map((service) => ({
      name: service.name,
      description: service.description,
      enabled: service.enabled,
      methods: service.methods.map((method) => ({
        name: method.name,
        description: method.description,
        enabled: method.enabled,
        input_schema_text: prettyJson(method.input_schema),
        binding:
          method.binding.type === "shell_command"
            ? {
                type: "shell_command",
                root_dir: method.binding.root_dir,
                allow_commands_text: method.binding.allow_commands.join(", "),
                default_timeout_secs: toOptionalText(method.binding.default_timeout_secs),
                max_timeout_secs: toOptionalText(method.binding.max_timeout_secs)
              }
            : {
                type: "http",
                url: method.binding.url,
                http_method: method.binding.http_method,
                headers_text: headersToText(method.binding.headers),
                timeout_secs: toOptionalText(method.binding.timeout_secs)
              }
      }))
    }))
  };
}

function fromUiConfig(config: UiAgentConfig): AgentConfig {
  return {
    platform: {
      base_url: normalizePlatformBaseUrl(config.platform.base_url),
      workspace_id: toOptionalNumber(config.platform.workspace_id)
    },
    relay: config.relay,
    device: {
      name: config.device.name.trim(),
      description: config.device.description.trim(),
      tags: splitCommaList(config.device.tags_text)
    },
    runtime: config.runtime,
    services: config.services.map((service) => ({
      name: service.name.trim(),
      description: service.description.trim(),
      enabled: service.enabled,
      methods: service.methods.map((method) => ({
        name: method.name.trim(),
        description: method.description.trim(),
        enabled: method.enabled,
        input_schema: parseJson(method.input_schema_text),
        binding:
          method.binding.type === "shell_command"
            ? {
                type: "shell_command",
                root_dir: method.binding.root_dir.trim(),
                allow_commands: splitCommaList(method.binding.allow_commands_text),
                default_timeout_secs: toOptionalNumber(method.binding.default_timeout_secs),
                max_timeout_secs: toOptionalNumber(method.binding.max_timeout_secs)
              }
            : {
                type: "http",
                url: method.binding.url.trim(),
                http_method: method.binding.http_method.trim().toUpperCase(),
                headers: textToHeaders(method.binding.headers_text),
                timeout_secs: toOptionalNumber(method.binding.timeout_secs)
              }
      }))
    }))
  };
}

function normalizePlatformBaseUrl(value: string): string {
  const normalized = value.trim();
  return normalized || DEFAULT_PLATFORM_BASE_URL;
}

function createShellMethod(): UiMethodConfig {
  return {
    name: "shellExec",
    description: "Run one allowlisted command with optional cwd and env.",
    enabled: true,
    input_schema_text: prettyJson(SHELL_SCHEMA),
    binding: {
      type: "shell_command",
      root_dir: ".",
      allow_commands_text: DEFAULT_SAFE_COMMANDS,
      default_timeout_secs: "",
      max_timeout_secs: ""
    }
  };
}

function createHttpMethod(): UiMethodConfig {
  return {
    name: "invokeApi",
    description: "Forward invocation arguments to a local HTTP endpoint.",
    enabled: true,
    input_schema_text: prettyJson(HTTP_SCHEMA),
    binding: {
      type: "http",
      url: "http://127.0.0.1:8081/api/invoke",
      http_method: "POST",
      headers_text: "",
      timeout_secs: ""
    }
  };
}

function splitCommaList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function isFullShellAccess(binding: UiShellBinding): boolean {
  return splitCommaList(binding.allow_commands_text).includes(FULL_ACCESS_COMMAND);
}

function headersToText(headers: Record<string, string>): string {
  return Object.entries(headers)
    .map(([key, value]) => `${key}: ${value}`)
    .join("\n");
}

function textToHeaders(value: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const line of value.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }
    const separator = trimmed.indexOf(":");
    if (separator <= 0) {
      throw new Error(`无效请求头: ${trimmed}`);
    }
    const key = trimmed.slice(0, separator).trim();
    const headerValue = trimmed.slice(separator + 1).trim();
    result[key] = headerValue;
  }
  return result;
}

function parseJson(text: string): unknown {
  return JSON.parse(text);
}

function prettyJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function safeNumber(value: string, fallback: number): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function toOptionalText(value?: number | null): string {
  return value == null ? "" : String(value);
}

function toOptionalNumber(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) {
    throw new Error(`无效数字: ${value}`);
  }
  return parsed;
}

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleString("zh-CN", {
    hour12: false
  });
}

function readError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export default App;
