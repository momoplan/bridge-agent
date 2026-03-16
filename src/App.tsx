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

  useEffect(() => {
    void refreshAll();
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void refreshRuntime();
    }, 1500);
    return () => window.clearInterval(timer);
  }, []);

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
      setMessage(`已打开浏览器授权页，用户码 ${session.userCode}`);
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
            这台机器上的业务服务、shell 能力和本地 HTTP 方法，都在这里定义并受控。
          </p>
        </div>
        <div className="hero-panel">
          <div className={`status-pill status-${runtime?.status ?? "stopped"}`}>
            {statusLabel}
          </div>
          <div className="hero-metrics">
            <div>
              <span>Agent ID</span>
              <strong>{runtime?.agent_id ?? config.relay.agent_id}</strong>
            </div>
            <div>
              <span>Relay</span>
              <strong>{runtime?.relay_url ?? config.relay.url}</strong>
            </div>
            <div>
              <span>配置文件</span>
              <strong>{configPath}</strong>
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
          {message ? <div className="alert success">{message}</div> : null}
          {error ? <div className="alert error">{error}</div> : null}
          {runtime?.last_error ? <div className="alert warning">{runtime.last_error}</div> : null}
          {browserAuth ? (
            <div className="alert warning">
              等待浏览器授权中，用户码 {browserAuth.userCode}
            </div>
          ) : null}
        </div>
      </section>

      <section className="workspace-grid">
        <div className="column-main">
          <Card title="连接设置" description="本地 agent 到 relay 的长连接参数。">
            <div className="form-grid">
              <Field label="Baijimu Base URL">
                <input
                  value={config.platform.base_url}
                  onChange={(event) => updatePlatform("base_url", event.target.value)}
                />
              </Field>
              <Field label="Workspace ID">
                <input
                  value={config.platform.workspace_id}
                  onChange={(event) => updatePlatform("workspace_id", event.target.value)}
                  placeholder="1106"
                />
              </Field>
              <Field label="Relay WebSocket URL">
                <input
                  value={config.relay.url}
                  onChange={(event) => updateRelay("url", event.target.value)}
                />
              </Field>
              <Field label="Agent ID">
                <input
                  value={config.relay.agent_id}
                  onChange={(event) => updateRelay("agent_id", event.target.value)}
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
          </Card>

          <Card title="设备资料" description="用于管理本机实例以及后续与平台设备记录对齐。">
            <div className="form-grid">
              <Field label="设备名">
                <input
                  value={config.device.name}
                  onChange={(event) => updateDevice("name", event.target.value)}
                />
              </Field>
              <Field label="标签">
                <input
                  value={config.device.tags_text}
                  onChange={(event) => updateDevice("tags_text", event.target.value)}
                  placeholder="desktop, local"
                />
              </Field>
              <Field label="设备描述" wide>
                <textarea
                  rows={3}
                  value={config.device.description}
                  onChange={(event) => updateDevice("description", event.target.value)}
                />
              </Field>
            </div>
          </Card>

          <Card title="运行策略" description="超时和日志保留上限都在本地生效。">
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
              <Field label="日志上限">
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
          </Card>

          <Card
            title="业务服务"
            description="按业务组织服务与方法。shell/http 只是本地方法绑定，不进入 relay 协议。"
            action={
              <button className="secondary" onClick={addService}>
                新增服务
              </button>
            }
          >
            <div className="service-list">
              {config.services.map((service, serviceIndex) => (
                <div className="service-card" key={`${service.name}-${serviceIndex}`}>
                  <div className="service-head">
                    <div>
                      <h3>{service.name || "未命名服务"}</h3>
                      <p>{service.description || "填写服务说明。"}</p>
                    </div>
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
                    <button className="secondary" onClick={() => addMethod(serviceIndex, "http")}>
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
                          <div className="form-grid">
                            <Field label="根目录">
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
                            <Field label="允许命令">
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
                                placeholder="echo, pwd, git"
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
              ))}
            </div>
          </Card>
        </div>

        <div className="column-side">
          <Card title="服务清单预览" description="这里展示将对外暴露的 manifest 形态。">
            <pre className="code-panel">{manifestPreview}</pre>
          </Card>

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
        </div>
      </section>
    </main>
  );
}

function Field(props: {
  label: string;
  children: JSX.Element;
  wide?: boolean;
}) {
  return (
    <label className={`field ${props.wide ? "field-wide" : ""}`}>
      <span>{props.label}</span>
      {props.children}
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

function toUiConfig(config: AgentConfig): UiAgentConfig {
  return {
    platform: {
      base_url: config.platform.base_url,
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
      base_url: config.platform.base_url.trim(),
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

function createShellMethod(): UiMethodConfig {
  return {
    name: "shellExec",
    description: "Run one allowlisted command with optional cwd and env.",
    enabled: true,
    input_schema_text: prettyJson(SHELL_SCHEMA),
    binding: {
      type: "shell_command",
      root_dir: ".",
      allow_commands_text: "echo, pwd, ls",
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
