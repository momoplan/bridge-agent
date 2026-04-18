import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState, type ReactNode } from "react";

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

interface UploadConfig {
  prepare_url?: string | null;
  inline_limit_bytes: number;
  timeout_secs: number;
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

type ComputerAction =
  | "screenshot"
  | "click"
  | "double_click"
  | "scroll"
  | "type"
  | "wait"
  | "keypress"
  | "drag"
  | "move";

interface ComputerUseBinding {
  type: "computer_use";
  action: ComputerAction;
  display_id?: number | null;
}

type MethodBinding = ShellBinding | HttpBinding | ComputerUseBinding;

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
  upload: UploadConfig;
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

interface AppUpdateStatus {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  releaseUrl: string;
  releaseName: string | null;
  publishedAt: string | null;
  currentTarget: string;
  autoDownloadAvailable: boolean;
  assetName: string | null;
}

interface AppUpdateInstallResult {
  status: "up_to_date" | "downloaded";
  version: string;
  assetName: string | null;
  downloadedPath: string | null;
  releaseUrl: string;
}

interface DesktopPermissionStatus {
  platform: string;
  accessibilityGranted: boolean;
  screenRecordingGranted: boolean;
  accessibilitySupported: boolean;
  screenRecordingSupported: boolean;
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

interface UiComputerUseBinding {
  type: "computer_use";
  action: ComputerAction;
  display_id: string;
}

type UiMethodBinding = UiShellBinding | UiHttpBinding | UiComputerUseBinding;

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
  upload: {
    prepare_url: string;
    inline_limit_bytes: number;
    timeout_secs: number;
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
type AppPage = "overview" | "services" | "connection" | "diagnostics";
type DetailPanel = "system" | "logs" | "manifest";

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

const EMPTY_OBJECT_SCHEMA = {
  type: "object",
  additionalProperties: false,
  properties: {}
};

const COMPUTER_MOUSE_SCHEMA = {
  type: "object",
  required: ["x", "y"],
  properties: {
    x: { type: "number" },
    y: { type: "number" },
    button: {
      type: "string",
      enum: ["left", "middle", "right"]
    },
    keys: {
      type: "array",
      items: { type: "string" }
    }
  }
};

const COMPUTER_SCROLL_SCHEMA = {
  type: "object",
  required: ["x", "y"],
  properties: {
    x: { type: "number" },
    y: { type: "number" },
    scroll_x: { type: "integer" },
    scroll_y: { type: "integer" },
    scrollX: { type: "integer" },
    scrollY: { type: "integer" },
    keys: {
      type: "array",
      items: { type: "string" }
    }
  }
};

const COMPUTER_TYPE_SCHEMA = {
  type: "object",
  required: ["text"],
  properties: {
    text: { type: "string" }
  }
};

const COMPUTER_WAIT_SCHEMA = {
  type: "object",
  properties: {
    ms: {
      type: "integer",
      minimum: 0
    }
  }
};

const COMPUTER_KEYPRESS_SCHEMA = {
  type: "object",
  required: ["keys"],
  properties: {
    keys: {
      type: "array",
      items: { type: "string" },
      minItems: 1
    }
  }
};

const COMPUTER_DRAG_SCHEMA = {
  type: "object",
  required: ["path"],
  properties: {
    path: {
      type: "array",
      minItems: 2,
      items: {
        type: "object",
        required: ["x", "y"],
        properties: {
          x: { type: "number" },
          y: { type: "number" }
        }
      }
    },
    keys: {
      type: "array",
      items: { type: "string" }
    }
  }
};

const DEFAULT_PLATFORM_BASE_URL = "https://baijimu.com/lowcode3";
const DEFAULT_SAFE_COMMANDS = "echo, pwd, ls, git";
const FULL_ACCESS_COMMAND = "*";
const FULL_ACCESS_ROOT_DIR = "/";

const COMPUTER_METHOD_PRESETS: Record<
  ComputerAction,
  { name: string; description: string; schema: unknown; label: string }
> = {
  screenshot: {
    name: "screenshot",
    description: "Capture the current desktop and return a PNG screenshot.",
    schema: EMPTY_OBJECT_SCHEMA,
    label: "截图"
  },
  click: {
    name: "click",
    description: "Click at a screen coordinate with an optional mouse button.",
    schema: COMPUTER_MOUSE_SCHEMA,
    label: "单击"
  },
  double_click: {
    name: "double_click",
    description: "Double-click at a screen coordinate.",
    schema: COMPUTER_MOUSE_SCHEMA,
    label: "双击"
  },
  scroll: {
    name: "scroll",
    description: "Scroll at a screen coordinate with horizontal and vertical deltas.",
    schema: COMPUTER_SCROLL_SCHEMA,
    label: "滚动"
  },
  type: {
    name: "type",
    description: "Type text into the currently focused app.",
    schema: COMPUTER_TYPE_SCHEMA,
    label: "输入文本"
  },
  wait: {
    name: "wait",
    description: "Pause briefly to let the desktop settle before the next screenshot.",
    schema: COMPUTER_WAIT_SCHEMA,
    label: "等待"
  },
  keypress: {
    name: "keypress",
    description: "Press one key or a key chord such as Command+L.",
    schema: COMPUTER_KEYPRESS_SCHEMA,
    label: "按键"
  },
  drag: {
    name: "drag",
    description: "Drag the pointer across a path of coordinates.",
    schema: COMPUTER_DRAG_SCHEMA,
    label: "拖拽"
  },
  move: {
    name: "move",
    description: "Move the pointer to a screen coordinate.",
    schema: COMPUTER_MOUSE_SCHEMA,
    label: "移动"
  }
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
  const [appUpdate, setAppUpdate] = useState<AppUpdateStatus | null>(null);
  const [desktopPermissions, setDesktopPermissions] = useState<DesktopPermissionStatus | null>(null);
  const [dismissedUpdateVersion, setDismissedUpdateVersion] = useState<string | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [desktopPermissionBusy, setDesktopPermissionBusy] = useState<"accessibility" | "screen_recording" | null>(
    null
  );
  const [expandedMethodAdvancedKey, setExpandedMethodAdvancedKey] = useState<string | null>(null);
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [activeSettingsSection, setActiveSettingsSection] =
    useState<SettingsSection>("identity");
  const [activePage, setActivePage] = useState<AppPage>("overview");
  const [activeDetailPanel, setActiveDetailPanel] = useState<DetailPanel>("system");
  const [expandedServiceIndex, setExpandedServiceIndex] = useState<number | null>(0);

  useEffect(() => {
    void refreshAll();
  }, []);

  useEffect(() => {
    void checkAppUpdate();
  }, []);

  useEffect(() => {
    void refreshDesktopPermissions();
  }, []);

  useEffect(() => {
    const handleWindowFocus = () => {
      void refreshDesktopPermissions();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshDesktopPermissions();
      }
    };

    window.addEventListener("focus", handleWindowFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("focus", handleWindowFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
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
  const enabledComputerMethodCount =
    config?.services.reduce(
      (count, service) =>
        count +
        (service.enabled
          ? service.methods.filter(
              (method) => method.enabled && method.binding.type === "computer_use"
            ).length
          : 0),
      0
    ) ?? 0;
  const selectedServiceIndex =
    expandedServiceIndex != null && config?.services[expandedServiceIndex] ? expandedServiceIndex : null;
  const selectedService = selectedServiceIndex == null ? null : config?.services[selectedServiceIndex] ?? null;
  const visibleAppUpdate =
    appUpdate?.updateAvailable && appUpdate.latestVersion !== dismissedUpdateVersion ? appUpdate : null;
  const hasDesktopPermissionGap =
    enabledComputerMethodCount > 0 &&
    desktopPermissions != null &&
    ((!desktopPermissions.accessibilityGranted && desktopPermissions.accessibilitySupported) ||
      (!desktopPermissions.screenRecordingGranted && desktopPermissions.screenRecordingSupported));

  function buildMethodEditorKey(serviceIndex: number, methodIndex: number) {
    return `${serviceIndex}:${methodIndex}`;
  }

  function isMethodAdvancedOpen(serviceIndex: number, methodIndex: number) {
    return expandedMethodAdvancedKey === buildMethodEditorKey(serviceIndex, methodIndex);
  }

  function toggleMethodAdvanced(serviceIndex: number, methodIndex: number) {
    const key = buildMethodEditorKey(serviceIndex, methodIndex);
    setExpandedMethodAdvancedKey((current) => (current === key ? null : key));
  }

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

  async function refreshDesktopPermissions() {
    try {
      const status = await invoke<DesktopPermissionStatus>("desktop_permission_status");
      setDesktopPermissions(status);
    } catch (err) {
      console.warn("读取桌面权限状态失败", err);
    }
  }

  async function checkAppUpdate(showLatestMessage = false) {
    try {
      const status = await invoke<AppUpdateStatus>("check_app_update");
      setAppUpdate(status);
      if (showLatestMessage) {
        setMessage(
          status.updateAvailable
            ? status.autoDownloadAvailable
              ? `发现新版本 ${status.latestVersion}，可以直接自动下载并安装。`
              : `发现新版本 ${status.latestVersion}，但当前平台需要跳转发布页手工下载。`
            : `当前已经是最新版本 ${status.currentVersion}`
        );
      }
      if (!status.updateAvailable) {
        setDismissedUpdateVersion(null);
      }
    } catch (err) {
      if (showLatestMessage) {
        setError(readError(err));
      } else {
        console.warn("自动检查更新失败", err);
      }
    }
  }

  async function installAppUpdate() {
    try {
      setUpdateBusy(true);
      setMessage("");
      setError("");
      const result = await invoke<AppUpdateInstallResult>("install_app_update");
      if (result.status === "up_to_date") {
        setMessage(`当前已经是最新版本 ${result.version}`);
        return;
      }
      setDismissedUpdateVersion(result.version);
      setMessage(
        result.downloadedPath
          ? `更新包 ${result.assetName ?? ""} 已下载到 ${result.downloadedPath}，并已打开安装。`
          : `更新包 ${result.assetName ?? ""} 已下载并开始安装。`
      );
    } catch (err) {
      setError(readError(err));
    } finally {
      setUpdateBusy(false);
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

  async function openExternalUrl(url: string) {
    try {
      await invoke("open_in_browser", { url });
    } catch (err) {
      setError(readError(err));
    }
  }

  async function requestDesktopPermission(permission: "accessibility" | "screen_recording") {
    try {
      setDesktopPermissionBusy(permission);
      setMessage("");
      setError("");
      const status = await invoke<DesktopPermissionStatus>("request_desktop_permission", {
        permission
      });
      setDesktopPermissions(status);
      if (permission === "screen_recording") {
        setMessage(
          status.screenRecordingGranted
            ? "屏幕录制权限已可用。"
            : "已请求屏幕录制权限。如果系统里已经允许但这里还没同步，先切回应用；少数情况下需要完全退出后重新打开。"
        );
      } else {
        setMessage(
          status.accessibilityGranted
            ? "桌面控制权限已可用。"
            : "已请求桌面控制权限。允许后切回应用会自动刷新；如果仍未同步，再完全退出后重开。"
        );
      }
    } catch (err) {
      setError(readError(err));
    } finally {
      setDesktopPermissionBusy(null);
    }
  }

  async function openDesktopPermissionSettings(permission: "accessibility" | "screen_recording") {
    try {
      setError("");
      await invoke("open_desktop_permission_settings", { permission });
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

  function updateUpload<K extends "prepare_url" | "inline_limit_bytes" | "timeout_secs">(
    key: K,
    value: UiAgentConfig["upload"][K]
  ) {
    setConfig((current) =>
      current
        ? {
            ...current,
            upload: {
              ...current.upload,
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
            <Field label="设备名称" hint="显示给平台和授权页。">
              <input
                value={config.device.name}
                onChange={(event) => updateDevice("name", event.target.value)}
              />
            </Field>
            <Field label="运行名称" hint="Relay 侧的唯一标识。默认会自动生成唯一值，不建议多台机器共用。">
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
            <Field label="配置文件" hint="相对路径都基于这里解析。">
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
                hint="默认使用正式环境。"
              >
                <input value={DEFAULT_PLATFORM_BASE_URL} readOnly />
              </Field>
              <Field
                label="授权后工作区"
                hint="浏览器授权成功后自动写回。"
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
                  hint="仅在测试环境修改。"
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
              <Field
                label="上传准备接口"
                wide
                hint="大截图会先调用这个接口申请上传槽位。留空时默认使用 relay 同域的 /api/bridge-agent/uploads/prepare。"
              >
                <input
                  value={config.upload.prepare_url}
                  onChange={(event) => updateUpload("prepare_url", event.target.value)}
                  placeholder="https://relay.baijimu.com/api/bridge-agent/uploads/prepare"
                />
              </Field>
              <Field
                label="内联上限字节"
                hint="截图超过这个阈值后不再直接走 WebSocket 内联。"
              >
                <input
                  type="number"
                  min={1024}
                  value={config.upload.inline_limit_bytes}
                  onChange={(event) =>
                    updateUpload("inline_limit_bytes", safeNumber(event.target.value, 8 * 1024 * 1024))
                  }
                />
              </Field>
              <Field label="上传超时秒数">
                <input
                  type="number"
                  min={1}
                  value={config.upload.timeout_secs}
                  onChange={(event) =>
                    updateUpload("timeout_secs", safeNumber(event.target.value, 60))
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
            <Field label="日志上限" hint="仅保留本地日志。">
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

function renderOverviewPage() {
    if (!config) {
      return <div />;
    }

    return (
      <div className="overview-grid">
        <Card title="运行">
          <div className="overview-stack">
            <div className="overview-status-row">
              <div className={`status-pill status-${runtime?.status ?? "stopped"}`}>{statusLabel}</div>
              <div className="overview-meta">
                <span>最近事件</span>
                <strong>{runtime ? formatTime(runtime.last_event_at) : "-"}</strong>
              </div>
            </div>
            <div className="hero-actions compact-actions">
              <button className="primary accent" onClick={() => void startAgent()} disabled={busy}>
                启动
              </button>
              <button className="secondary" onClick={() => void stopAgent()} disabled={busy}>
                停止
              </button>
              <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
                重新授权
              </button>
            </div>
          </div>
        </Card>

        <Card title="设备">
          <div className="status-detail-grid">
            <InfoRow label="设备名称" value={config.device.name} />
            <InfoRow label="运行名称" value={runtime?.agent_id ?? config.relay.agent_id} />
            <InfoRow label="工作区" value={config.platform.workspace_id || "未授权"} />
          </div>
        </Card>

        <Card title="连接">
          <div className="status-detail-grid">
            <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
            <InfoRow
              label="最近错误"
              value={runtime?.last_error || "无"}
              tone={runtime?.last_error ? "danger" : "normal"}
            />
            <InfoRow
              label="更新"
              value={
                appUpdate?.updateAvailable
                  ? `可升级到 ${appUpdate.latestVersion ?? "-"}`
                  : appUpdate
                    ? "已是最新版本"
                    : "检查中"
              }
              tone={appUpdate?.updateAvailable ? "danger" : "normal"}
            />
          </div>
        </Card>

        <Card
          title="桌面权限"
          action={
            <button className="ghost" onClick={() => void refreshDesktopPermissions()}>
              刷新状态
            </button>
          }
        >
          <div className="status-detail-grid">
            <InfoRow
              label="屏幕录制"
              value={formatDesktopPermissionValue(
                desktopPermissions,
                "screen_recording",
                "用于截图"
              )}
              tone={
                desktopPermissions?.screenRecordingSupported &&
                !desktopPermissions.screenRecordingGranted
                  ? "danger"
                  : "normal"
              }
            />
            <InfoRow
              label="辅助功能"
              value={formatDesktopPermissionValue(
                desktopPermissions,
                "accessibility",
                "用于点击、输入和拖拽"
              )}
              tone={
                desktopPermissions?.accessibilitySupported &&
                !desktopPermissions.accessibilityGranted
                  ? "danger"
                  : "normal"
              }
            />
          </div>
          {desktopPermissions?.platform === "macos" ? (
            <div className="permission-actions">
              {!desktopPermissions.screenRecordingGranted ? (
                <button
                  className="secondary"
                  onClick={() => void requestDesktopPermission("screen_recording")}
                  disabled={desktopPermissionBusy != null}
                >
                  {desktopPermissionBusy === "screen_recording" ? "请求中" : "请求屏幕录制"}
                </button>
              ) : null}
              {!desktopPermissions.accessibilityGranted ? (
                <button
                  className="secondary"
                  onClick={() => void requestDesktopPermission("accessibility")}
                  disabled={desktopPermissionBusy != null}
                >
                  {desktopPermissionBusy === "accessibility" ? "请求中" : "请求辅助功能"}
                </button>
              ) : null}
              {!desktopPermissions.screenRecordingGranted ? (
                <button
                  className="ghost"
                  onClick={() => void openDesktopPermissionSettings("screen_recording")}
                >
                  打开屏幕录制设置
                </button>
              ) : null}
              {!desktopPermissions.accessibilityGranted ? (
                <button
                  className="ghost"
                  onClick={() => void openDesktopPermissionSettings("accessibility")}
                >
                  打开辅助功能设置
                </button>
              ) : null}
            </div>
          ) : null}
        </Card>

        <Card
          title="能力"
          action={
            <button className="secondary" onClick={() => setActivePage("services")}>
              打开能力页
            </button>
          }
        >
          <div className="status-detail-grid">
            <InfoRow label="总能力数" value={String(config.services.length)} />
            <InfoRow label="已启用" value={String(enabledServiceCount)} />
            <InfoRow label="最近日志" value={latestLog ? formatTime(latestLog.timestamp_ms) : "暂无"} />
          </div>
        </Card>
      </div>
    );
  }

  function renderServiceEditor(service: UiServiceConfig, serviceIndex: number) {
    const isComputer = isComputerService(service);

    return (
      <Card
        title={service.name || "未命名服务"}
        description={
          isComputer
            ? "系统内置"
            : service.description || "自定义本地能力"
        }
        action={
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
            {!isComputer ? (
              <button className="ghost danger" onClick={() => removeService(serviceIndex)}>
                删除服务
              </button>
            ) : null}
          </div>
        }
      >
        <div className="service-editor-layout">
          {isComputer ? (
            <div className="service-readonly-banner">
              <strong>系统能力</strong>
              <p>`computer` 由应用自动维护，只展示可用动作，不做方法级编辑。</p>
            </div>
          ) : (
            <>
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
                <button className="secondary" onClick={() => addMethod(serviceIndex, "shell_command")}>
                  新增 Shell 方法
                </button>
                <button className="secondary" onClick={() => addMethod(serviceIndex, "http")}>
                  新增 HTTP 方法
                </button>
              </div>
            </>
          )}
          <div className="method-list">
            {service.methods.map((method, methodIndex) => (
              <div className="method-card" key={`${service.name}-${method.name}-${methodIndex}`}>
                <div className="method-topline">
                  <div className="method-copy">
                    <div className="method-title-row">
                      <h4>{method.name || "未命名方法"}</h4>
                      <span className="method-badge">{formatMethodTypeLabel(method.binding.type)}</span>
                      {isComputer ? (
                        <span className={`service-badge ${method.enabled ? "enabled" : "disabled"}`}>
                          {method.enabled ? "启用" : "停用"}
                        </span>
                      ) : null}
                    </div>
                    <p>{describeMethodBinding(method)}</p>
                    {isComputer && method.binding.type === "computer_use" ? (
                      <div className="method-facts">
                        <span>动作：{COMPUTER_METHOD_PRESETS[method.binding.action].label}</span>
                        <span>说明：{method.description || COMPUTER_METHOD_PRESETS[method.binding.action].description}</span>
                      </div>
                    ) : null}
                  </div>
                  {!isComputer ? (
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
                        className={isMethodAdvancedOpen(serviceIndex, methodIndex) ? "secondary" : "ghost"}
                        onClick={() => toggleMethodAdvanced(serviceIndex, methodIndex)}
                      >
                        {isMethodAdvancedOpen(serviceIndex, methodIndex) ? "收起高级设置" : "高级设置"}
                      </button>
                      <button
                        className="ghost danger"
                        onClick={() => removeMethod(serviceIndex, methodIndex)}
                      >
                        删除方法
                      </button>
                    </div>
                  ) : null}
                </div>

                {!isComputer ? (
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
                    <Field label="绑定类型">
                      <select
                        value={method.binding.type}
                        disabled={method.binding.type === "computer_use"}
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
                        {method.binding.type === "computer_use" ? (
                          <option value="computer_use">computer_use（内置）</option>
                        ) : null}
                        <option value="shell_command">shell_command</option>
                        <option value="http">http</option>
                      </select>
                    </Field>

                    {method.binding.type === "computer_use" ? (
                      <Field
                        label="桌面能力"
                        hint="内置能力由系统维护，不在普通服务里编辑。"
                      >
                        <input value={COMPUTER_METHOD_PRESETS[method.binding.action].label} readOnly />
                      </Field>
                    ) : method.binding.type === "shell_command" ? (
                      <Field label="权限模式">
                        <div className="mode-toggle">
                          <button
                            className={!isFullShellAccess(method.binding) ? "secondary active-toggle" : "ghost"}
                            onClick={() => restoreSafeShellAccess(serviceIndex, methodIndex)}
                          >
                            受限
                          </button>
                          <button
                            className={isFullShellAccess(method.binding) ? "secondary active-toggle" : "ghost"}
                            onClick={() => grantFullShellAccess(serviceIndex, methodIndex)}
                          >
                            全部权限
                          </button>
                        </div>
                      </Field>
                    ) : (
                      <>
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
                      </>
                    )}
                  </div>
                ) : null}

                {!isComputer && isMethodAdvancedOpen(serviceIndex, methodIndex) ? (
                  <div className="method-advanced">
                    <div className="method-advanced-head">
                      <strong>高级设置</strong>
                      <small>只有在需要细调本地实现时才展开这里。</small>
                    </div>

                    {method.binding.type === "shell_command" ? (
                      <>
                        <div className="permission-banner compact-banner">
                          <div>
                            <strong>
                              {isFullShellAccess(method.binding) ? "当前是全部权限模式" : "当前是受限模式"}
                            </strong>
                            <p>这里决定命令白名单、根目录和超时上限。</p>
                          </div>
                        </div>
                        <div className="form-grid">
                          <Field label="根目录" hint="默认相对配置目录，填 / 表示整机根目录。">
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
                          <Field label="允许命令" hint="逗号分隔；填 * 表示任意命令。">
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
                    ) : method.binding.type === "computer_use" ? (
                      <div className="form-grid">
                        <Field
                          label="显示器 ID"
                          hint="留空表示主显示器；截图时可指定其他显示器。"
                        >
                          <input
                            value={method.binding.display_id}
                            onChange={(event) =>
                              updateMethod(serviceIndex, methodIndex, (current) => ({
                                ...current,
                                binding:
                                  current.binding.type === "computer_use"
                                    ? {
                                        ...current.binding,
                                        display_id: event.target.value
                                      }
                                    : current.binding
                              }))
                            }
                            placeholder="留空使用主屏"
                          />
                        </Field>
                      </div>
                    ) : (
                      <div className="form-grid">
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
                ) : null}
              </div>
            ))}
          </div>
        </div>
      </Card>
    );
  }

  function renderServicesPage() {
    if (!config) {
      return <div />;
    }

    const systemServices = config.services
      .map((service, serviceIndex) => ({ service, serviceIndex }))
      .filter(({ service }) => isComputerService(service));
    const customServices = config.services
      .map((service, serviceIndex) => ({ service, serviceIndex }))
      .filter(({ service }) => !isComputerService(service));

    return (
      <div className="services-layout">
        <Card
          title="能力"
          description="左侧选择能力，右侧查看或调整。"
          action={
            <button className="secondary" onClick={addService}>
              新增自定义服务
            </button>
          }
        >
          <div className="service-nav-list">
            {config.services.length === 0 ? (
              <div className="empty-state">还没有能力，先新增一个。</div>
            ) : (
              <>
                <div className="service-nav-section">系统能力</div>
                {systemServices.map(({ service, serviceIndex }) => (
                  <button
                    className={`service-nav-item ${selectedServiceIndex === serviceIndex ? "active" : ""}`}
                    key={`${service.name}-${serviceIndex}`}
                    onClick={() => setExpandedServiceIndex(serviceIndex)}
                  >
                    <div>
                      <strong>{service.name || "未命名服务"}</strong>
                      <p>{describeServiceSummary(service)}</p>
                    </div>
                    <div className="service-nav-meta">
                      <span className={`service-badge ${service.enabled ? "enabled" : "disabled"}`}>
                        {service.enabled ? "启用" : "停用"}
                      </span>
                      <small>{service.methods.length} 项动作</small>
                    </div>
                  </button>
                ))}
                <div className="service-nav-section">自定义服务</div>
                {customServices.length === 0 ? (
                  <div className="empty-state compact-empty-state">还没有自定义服务。</div>
                ) : (
                  customServices.map(({ service, serviceIndex }) => (
                    <button
                      className={`service-nav-item ${selectedServiceIndex === serviceIndex ? "active" : ""}`}
                      key={`${service.name}-${serviceIndex}`}
                      onClick={() => setExpandedServiceIndex(serviceIndex)}
                    >
                      <div>
                        <strong>{service.name || "未命名服务"}</strong>
                        <p>{describeServiceSummary(service)}</p>
                      </div>
                      <div className="service-nav-meta">
                        <span className={`service-badge ${service.enabled ? "enabled" : "disabled"}`}>
                          {service.enabled ? "启用" : "停用"}
                        </span>
                        <small>{service.methods.length} 个接口</small>
                      </div>
                    </button>
                  ))
                )}
              </>
            )}
          </div>
        </Card>

        <div className="service-editor-panel">
          {selectedService && selectedServiceIndex != null ? (
            renderServiceEditor(selectedService, selectedServiceIndex)
          ) : (
            <Card title="能力详情" description="从左侧选择一项开始。">
              <div className="empty-state">还没有可编辑的能力。</div>
            </Card>
          )}
        </div>
      </div>
    );
  }

  function renderConnectionPage() {
    return (
      <Card title="连接" description="设备、授权与运行参数。">
        <div className="status-detail-grid connection-summary-grid">
          <InfoRow label="工作区" value={config?.platform.workspace_id || "未授权"} />
          <InfoRow label="平台" value={DEFAULT_PLATFORM_BASE_URL} />
          <InfoRow label="Relay" value={runtime?.relay_url ?? config?.relay.url ?? "-"} />
        </div>
        <div className="section-tabs">
          <button
            className={`section-tab ${activeSettingsSection === "identity" ? "active" : ""}`}
            onClick={() => setActiveSettingsSection("identity")}
          >
            设备
          </button>
          <button
            className={`section-tab ${activeSettingsSection === "connection" ? "active" : ""}`}
            onClick={() => setActiveSettingsSection("connection")}
          >
            连接
          </button>
          <button
            className={`section-tab ${activeSettingsSection === "runtime" ? "active" : ""}`}
            onClick={() => setActiveSettingsSection("runtime")}
          >
            运行
          </button>
        </div>
        {renderSettingsSection()}
      </Card>
    );
  }

  function renderDetailPanel() {
    if (!config) {
      return <div />;
    }

    if (activeDetailPanel === "logs") {
      return (
        <Card
          title="日志"
          description="最近运行记录。"
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
        <Card title="对外清单" description="联调时查看。">
          <pre className="code-panel">{manifestPreview}</pre>
        </Card>
      );
    }

    return (
      <Card title="系统" description="版本与运行状态。">
        <div className="status-detail-grid">
          <InfoRow label="应用版本" value={appUpdate?.currentVersion ?? "检查中"} />
          <InfoRow
            label="更新状态"
            value={
              appUpdate?.updateAvailable
                ? `可升级到 ${appUpdate.latestVersion ?? "-"}`
                : appUpdate
                  ? "已是最新版本"
                  : "尚未完成检查"
            }
            tone={appUpdate?.updateAvailable ? "danger" : "normal"}
          />
          <InfoRow label="当前状态" value={statusLabel} />
          <InfoRow label="最近事件" value={runtime ? formatTime(runtime.last_event_at) : "-"} />
          <InfoRow label="运行名称" value={runtime?.agent_id ?? config.relay.agent_id} />
          <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
          <InfoRow
            label="桌面权限"
            value={
              desktopPermissions == null
                ? "检查中"
                : desktopPermissions.accessibilityGranted &&
                    desktopPermissions.screenRecordingGranted
                    ? "已就绪"
                    : desktopPermissions.accessibilitySupported ||
                        desktopPermissions.screenRecordingSupported
                      ? "权限未完整授权"
                      : "当前平台未接入"
            }
            tone={
              desktopPermissions != null &&
              (desktopPermissions.accessibilitySupported ||
                desktopPermissions.screenRecordingSupported) &&
              (!desktopPermissions.accessibilityGranted || !desktopPermissions.screenRecordingGranted)
                ? "danger"
                : "normal"
            }
          />
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

  function renderDiagnosticsPage() {
    return (
      <div className="diagnostics-layout">
        <div className="section-tabs">
          <button
            className={`section-tab ${activeDetailPanel === "system" ? "active" : ""}`}
            onClick={() => setActiveDetailPanel("system")}
          >
            系统
          </button>
          <button
            className={`section-tab ${activeDetailPanel === "logs" ? "active" : ""}`}
            onClick={() => setActiveDetailPanel("logs")}
          >
            日志
          </button>
          <button
            className={`section-tab ${activeDetailPanel === "manifest" ? "active" : ""}`}
            onClick={() => setActiveDetailPanel("manifest")}
          >
            清单
          </button>
        </div>
        {renderDetailPanel()}
      </div>
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
      <main className="app-shell app-loading">
        <section className="loading-panel">
          <p className="eyebrow">Bridge Agent</p>
          <h1>正在加载</h1>
          <p>读取配置和运行状态。</p>
          {error ? <div className="alert error">{error}</div> : null}
        </section>
      </main>
    );
  }

  const pageTitleMap: Record<AppPage, string> = {
    overview: "概览",
    services: "能力",
    connection: "连接",
    diagnostics: "诊断"
  };

  const pageDescriptionMap: Record<AppPage, string> = {
    overview: "",
    services: "系统能力与本地服务",
    connection: "连接、授权和运行参数",
    diagnostics: "系统、日志与清单"
  };

  return (
    <main className="app-shell">
      <div className="desktop-shell">
        <aside className="sidebar">
          <div className="sidebar-brand">
            <p className="eyebrow">Bridge Agent</p>
            <h1>{config.device.name}</h1>
            <div className={`status-pill status-${runtime?.status ?? "stopped"}`}>{statusLabel}</div>
          </div>

          <nav className="sidebar-nav">
            <button
              className={`sidebar-nav-item ${activePage === "overview" ? "active" : ""}`}
              onClick={() => setActivePage("overview")}
            >
              <span>概览</span>
              <small>状态与主操作</small>
            </button>
            <button
              className={`sidebar-nav-item ${activePage === "services" ? "active" : ""}`}
              onClick={() => setActivePage("services")}
            >
              <span>能力</span>
              <small>系统与自定义</small>
            </button>
            <button
              className={`sidebar-nav-item ${activePage === "connection" ? "active" : ""}`}
              onClick={() => setActivePage("connection")}
            >
              <span>连接</span>
              <small>{config.platform.workspace_id || "未授权"}</small>
            </button>
            <button
              className={`sidebar-nav-item ${activePage === "diagnostics" ? "active" : ""}`}
              onClick={() => setActivePage("diagnostics")}
            >
              <span>诊断</span>
              <small>{latestLog ? "查看日志与清单" : "系统信息"}</small>
            </button>
          </nav>

          <div className="sidebar-footer">
            <div className="sidebar-stat">
              <span>已启用服务</span>
              <strong>{enabledServiceCount}</strong>
            </div>
            <div className="sidebar-stat">
              <span>应用版本</span>
              <strong>{appUpdate?.currentVersion ?? "检查中"}</strong>
            </div>
          </div>
        </aside>

        <section className="main-panel">
          <header className="page-header">
            <div>
              <p className="eyebrow">{pageTitleMap[activePage]}</p>
              <h2>{pageTitleMap[activePage]}</h2>
              {pageDescriptionMap[activePage] ? <p>{pageDescriptionMap[activePage]}</p> : null}
            </div>
            <div className="page-actions">
              {activePage === "services" || activePage === "connection" ? (
                <button className="primary" onClick={() => void saveOnly()} disabled={busy}>
                  保存更改
                </button>
              ) : null}
              {activePage === "connection" ? (
                <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
                  浏览器授权
                </button>
              ) : null}
              {activePage === "diagnostics" ? (
                <button
                  className="secondary"
                  onClick={() => void checkAppUpdate(true)}
                  disabled={updateBusy}
                >
                  检查更新
                </button>
              ) : null}
              {activePage === "diagnostics" ? (
                <button className="ghost" onClick={() => void resetExampleConfig()} disabled={busy}>
                  恢复示例
                </button>
              ) : null}
            </div>
          </header>

          {visibleAppUpdate ? (
            <div className="update-banner">
              <div>
                <p className="update-banner-eyebrow">发现新版本</p>
                <strong>
                  当前 {visibleAppUpdate.currentVersion}，可升级到 {visibleAppUpdate.latestVersion}
                </strong>
                <p>
                  {visibleAppUpdate.releaseName || "GitHub Release"}
                  {visibleAppUpdate.publishedAt
                    ? `，发布于 ${formatReleaseTime(visibleAppUpdate.publishedAt)}`
                    : ""}
                </p>
              </div>
              <div className="update-banner-actions">
                {visibleAppUpdate.autoDownloadAvailable ? (
                  <button
                    className="primary"
                    onClick={() => void installAppUpdate()}
                    disabled={updateBusy}
                  >
                    自动更新
                  </button>
                ) : (
                  <button
                    className="primary"
                    onClick={() => void openExternalUrl(visibleAppUpdate.releaseUrl)}
                    disabled={updateBusy}
                  >
                    打开下载页
                  </button>
                )}
                <button
                  className="ghost"
                  onClick={() => setDismissedUpdateVersion(visibleAppUpdate.latestVersion)}
                  disabled={updateBusy}
                >
                  稍后提醒
                </button>
              </div>
            </div>
          ) : null}

          {hasDesktopPermissionGap ? (
            <div className="permission-banner">
              <div>
                <strong>桌面控制权限未准备完成</strong>
                <p>
                  当前已启用 {enabledComputerMethodCount} 个桌面控制方法，但
                  {!desktopPermissions?.screenRecordingGranted ? " 屏幕录制" : ""}
                  {!desktopPermissions?.screenRecordingGranted &&
                  !desktopPermissions?.accessibilityGranted
                    ? " 和"
                    : ""}
                  {!desktopPermissions?.accessibilityGranted ? " 辅助功能" : ""}
                  还没有授权。
                </p>
              </div>
              <div className="permission-actions">
                {!desktopPermissions?.screenRecordingGranted ? (
                  <button
                    className="secondary"
                    onClick={() => void openDesktopPermissionSettings("screen_recording")}
                  >
                    屏幕录制设置
                  </button>
                ) : null}
                {!desktopPermissions?.accessibilityGranted ? (
                  <button
                    className="secondary"
                    onClick={() => void openDesktopPermissionSettings("accessibility")}
                  >
                    辅助功能设置
                  </button>
                ) : null}
              </div>
            </div>
          ) : null}

          {message ? <div className="alert success">{message}</div> : null}
          {error ? <div className="alert error">{error}</div> : null}
          {runtime?.last_error && activePage !== "diagnostics" ? (
            <div className="alert warning">{runtime.last_error}</div>
          ) : null}
          {browserAuth ? (
            <div className="alert warning">等待浏览器授权中，用户码 {browserAuth.userCode}。</div>
          ) : null}

          <div className="page-body">
            {activePage === "overview" ? renderOverviewPage() : null}
            {activePage === "services" ? renderServicesPage() : null}
            {activePage === "connection" ? renderConnectionPage() : null}
            {activePage === "diagnostics" ? renderDiagnosticsPage() : null}
          </div>
        </section>
      </div>
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
  description?: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <section className="card">
      <div className="card-head">
        <div>
          <h2>{props.title}</h2>
          {props.description ? <p>{props.description}</p> : null}
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
    upload: {
      prepare_url: config.upload.prepare_url ?? "",
      inline_limit_bytes: config.upload.inline_limit_bytes,
      timeout_secs: config.upload.timeout_secs
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
            : method.binding.type === "http"
              ? {
                  type: "http",
                  url: method.binding.url,
                  http_method: method.binding.http_method,
                  headers_text: headersToText(method.binding.headers),
                  timeout_secs: toOptionalText(method.binding.timeout_secs)
                }
              : {
                  type: "computer_use",
                  action: method.binding.action,
                  display_id: toOptionalText(method.binding.display_id)
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
    upload: {
      prepare_url: emptyToNull(config.upload.prepare_url),
      inline_limit_bytes: config.upload.inline_limit_bytes,
      timeout_secs: config.upload.timeout_secs
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
            : method.binding.type === "http"
              ? {
                  type: "http",
                  url: method.binding.url.trim(),
                  http_method: method.binding.http_method.trim().toUpperCase(),
                  headers: textToHeaders(method.binding.headers_text),
                  timeout_secs: toOptionalNumber(method.binding.timeout_secs)
                }
              : {
                  type: "computer_use",
                  action: method.binding.action,
                  display_id: toOptionalNumber(method.binding.display_id)
                }
      }))
    }))
  };
}

function normalizePlatformBaseUrl(value: string): string {
  const normalized = value.trim();
  return normalized || DEFAULT_PLATFORM_BASE_URL;
}

function emptyToNull(value: string): string | null {
  const normalized = value.trim();
  return normalized ? normalized : null;
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

function isComputerService(service: Pick<UiServiceConfig, "name">): boolean {
  return service.name.trim().toLowerCase() === "computer";
}

function formatMethodTypeLabel(type: UiMethodBinding["type"]): string {
  if (type === "shell_command") {
    return "Shell";
  }
  if (type === "http") {
    return "HTTP";
  }
  return "Computer";
}

function describeServiceSummary(service: UiServiceConfig): string {
  if (isComputerService(service)) {
    return "桌面控制、截图、点击与输入";
  }

  const shellCount = service.methods.filter((method) => method.binding.type === "shell_command").length;
  const httpCount = service.methods.filter((method) => method.binding.type === "http").length;
  const summary: string[] = [];

  if (shellCount > 0) {
    summary.push(`Shell ${shellCount}`);
  }
  if (httpCount > 0) {
    summary.push(`HTTP ${httpCount}`);
  }

  return summary.length > 0 ? summary.join(" · ") : "尚未配置接口";
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

function describeMethodBinding(method: UiMethodConfig): string {
  if (method.binding.type === "shell_command") {
    return isFullShellAccess(method.binding) ? "Shell 调用，全部权限模式" : "Shell 调用，受限模式";
  }
  if (method.binding.type === "computer_use") {
    const displayLabel = method.binding.display_id.trim()
      ? `，显示器 ${method.binding.display_id}`
      : "";
    return `Computer use · ${method.binding.action}${displayLabel}`;
  }
  const url = method.binding.url.trim() || "未填写 URL";
  return `${method.binding.http_method || "HTTP"} ${url}`;
}

function formatDesktopPermissionValue(
  status: DesktopPermissionStatus | null,
  permission: "screen_recording" | "accessibility",
  unsupportedLabel: string
): string {
  if (!status) {
    return "检查中";
  }
  const supported =
    permission === "screen_recording" ? status.screenRecordingSupported : status.accessibilitySupported;
  if (!supported) {
    return `当前平台未接入，${unsupportedLabel}`;
  }
  if (permission === "screen_recording") {
    return status.screenRecordingGranted ? "已授权" : "未授权";
  }
  return status.accessibilityGranted ? "已授权" : "未授权";
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

function formatReleaseTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleDateString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit"
  });
}

function readError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export default App;
