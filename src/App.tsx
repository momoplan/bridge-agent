import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import bjmLogoLight from "./assets/brand/bjm-logo-light.svg";

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
  relay_registered: boolean;
  relay_registered_at: number | null;
  last_relay_seen_at: number | null;
  log_file_path: string | null;
  last_error: string | null;
  last_event_at: number;
}

interface RuntimeProcessInfo {
  pid: number;
  parent_pid: number | null;
  name: string | null;
  executable_path: string | null;
  command_line: string | null;
  running: boolean;
}

interface RuntimeLockConflict {
  pid: number;
  agent_id: string;
  config_path: string;
  lock_path: string;
  process: RuntimeProcessInfo;
}

type CommandError =
  | { code: "runtime_already_running"; conflict: RuntimeLockConflict }
  | { code: "message"; message: string };

interface LogEntry {
  timestamp_ms: number;
  level: string;
  message: string;
  category?: string;
  service?: string;
  method?: string;
  event?: string;
  request_id?: string;
  event_id?: string;
  outcome?: string;
  duration_ms?: number;
  http_method?: string;
  path?: string;
  status_code?: number;
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
  node_path?: string | null;
  codex_binary_path?: string | null;
  default_timeout_secs: number;
  max_timeout_secs: number;
  log_limit: number;
  log_file_enabled: boolean;
  log_file_dir?: string | null;
  log_file_max_bytes: number;
  log_file_max_files: number;
  event_server_enabled: boolean;
  event_server_bind: string;
  event_server_token?: string | null;
  service_registration_enabled: boolean;
  service_registration_token?: string | null;
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

interface EventConfig {
  name: string;
  description: string;
  enabled: boolean;
  payload_schema: unknown;
}

interface ServiceHealthCheckHttp {
  type: "http";
  url: string;
  http_method: string;
  headers: Record<string, string>;
  timeout_secs?: number | null;
  expect_status?: number | null;
  body_contains?: string | null;
}

type ServiceHealthCheck = ServiceHealthCheckHttp;

interface ServiceStartShellCommand {
  type: "shell_command";
  command: string[];
  cwd?: string | null;
  env: Record<string, string>;
  timeout_secs?: number | null;
}

type ServiceStartCommand = ServiceStartShellCommand;

interface ServiceConfig {
  name: string;
  description: string;
  enabled: boolean;
  health_check?: ServiceHealthCheck | null;
  start_command?: ServiceStartCommand | null;
  stop_command?: ServiceStartCommand | null;
  methods: MethodConfig[];
  events: EventConfig[];
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
  runtime: RuntimeSnapshot | null;
}

interface CapabilityInvokeError {
  code: string;
  message: string;
}

interface CapabilityInvokeResult {
  request_id: string;
  success: boolean;
  data?: unknown | null;
  error?: CapabilityInvokeError | null;
  duration_ms: number;
}

interface CapabilityTestState {
  status: "success" | "error";
  result?: CapabilityInvokeResult;
  message?: string;
}

interface ConfigDocument {
  config_path: string;
  manifest_preview: string;
  config: AgentConfig;
  runtime: RuntimeSnapshot;
}

interface ConfigRecoveryDocument extends ConfigDocument {
  archived_path: string | null;
}

interface AppUpdateStatus {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  forceUpdateRequired: boolean;
  minimumSupportedVersion: string | null;
  forceUpdateMessage: string | null;
  releaseUrl: string;
  releaseName: string | null;
  publishedAt: string | null;
  currentTarget: string;
  autoDownloadAvailable: boolean;
  assetName: string | null;
}

interface AppVersionInfo {
  currentVersion: string;
  currentTarget: string;
}

type AppUpdateCheckState = "checking" | "ready" | "error";

interface AppUpdateInstallResult {
  status: "up_to_date" | "downloaded";
  version: string;
  assetName: string | null;
  downloadedPath: string | null;
  releaseUrl: string;
}

type AppUpdateProgressPhase =
  | "checking"
  | "downloading"
  | "verifying"
  | "saving"
  | "scheduling"
  | "ready_to_install";

interface AppUpdateProgress {
  phase: AppUpdateProgressPhase;
  message: string;
  version: string | null;
  assetName: string | null;
  downloadedBytes: number | null;
  totalBytes: number | null;
  downloadedPath: string | null;
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

interface UiEventConfig {
  name: string;
  description: string;
  enabled: boolean;
  payload_schema_text: string;
}

interface UiServiceHealthCheckHttp {
  type: "http";
  url: string;
  http_method: string;
  headers_text: string;
  timeout_secs: string;
  expect_status: string;
  body_contains: string;
}

type UiServiceHealthCheck = UiServiceHealthCheckHttp;

interface UiServiceStartShellCommand {
  type: "shell_command";
  command_text: string;
  cwd: string;
  env_text: string;
  timeout_secs: string;
}

type UiServiceStartCommand = UiServiceStartShellCommand;

interface UiServiceConfig {
  name: string;
  description: string;
  enabled: boolean;
  health_check: UiServiceHealthCheck | null;
  start_command: UiServiceStartCommand | null;
  stop_command: UiServiceStartCommand | null;
  methods: UiMethodConfig[];
  events: UiEventConfig[];
}

interface ServiceCapabilitiesDocument {
  methods: MethodConfig[];
  events: EventConfig[];
}

type RegisteredServiceState = "not_configured" | "healthy" | "unhealthy" | "unknown";

interface RegisteredServiceStatus {
  service: string;
  status: RegisteredServiceState;
  detail: string | null;
  checkedAtMs: number;
  healthCheckConfigured: boolean;
  startCommandConfigured: boolean;
  stopCommandConfigured: boolean;
}

interface StartRegisteredServiceResult {
  service: string;
  success: boolean;
  exitCode: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

interface ConnectorSummary {
  id: string;
  name: string;
  version: string;
  packagePath: string;
  sourcePath: string;
  sourceReference?: string | null;
  serviceNames: string[];
  installedAtEpochMs: number;
  lastSyncedAtEpochMs: number;
}

interface ConnectorInstallResult {
  connectorId: string;
  name: string;
  version: string;
  packagePath: string;
  serviceNames: string[];
}

interface ConnectorAppInstallDocument {
  install: ConnectorInstallResult;
  config: ConfigDocument;
}

interface ConnectorServiceStartResult {
  service: string;
  configured: boolean;
  exitCode: number | null;
  stdout: string;
  stderr: string;
}

interface ConnectorStartResult {
  connectorId: string;
  services: ConnectorServiceStartResult[];
}

interface ConnectorAppUpdateStatus {
  connectorId: string;
  name: string;
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  source: string;
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
const DEFAULT_INLINE_LIMIT_BYTES = 256 * 1024;
const CODEX_CONNECTOR_ID = "com.baijimu.connector.codex";
type AppPage = "overview" | "apps" | "diagnostics";
type DetailPanel = "system" | "settings" | "logs" | "manifest";
type LocalAppKind = "connector" | "managed_tool" | "built_in" | "custom";
type InstallSourceMode = "choose" | "market" | "custom";
type LocalAppDetailTab = "overview" | "account" | "capabilities" | "config";
type LocalAppLifecycleState =
  | "installed"
  | "ready"
  | "missing"
  | "broken"
  | "updating"
  | "starting"
  | "running"
  | "start_failed"
  | "stopped"
  | "stopping"
  | "unknown";

interface LocalAppLifecycleOverride {
  state: LocalAppLifecycleState;
  detail?: string;
}

interface LocalAppLifecycle {
  state: LocalAppLifecycleState;
  label: string;
  detail: string;
  statusClass: string;
}

interface LocalAppItem {
  id: string;
  name: string;
  description: string;
  kind: LocalAppKind;
  serviceIndexes: number[];
  connector?: ConnectorSummary;
  managedTool?: ManagedToolStatus;
  codexAccountManagement?: boolean;
}

interface MarketConnector {
  id: string;
  connectorId: string;
  applicationType: string;
  name: string;
  description: string;
  source: string;
  checksum?: string | null;
  archivePath?: string | null;
  risk: string;
  riskLevel: string;
  capability: string;
  version: string;
}

interface ManagedToolStatus {
  id: string;
  name: string;
  description: string;
  state: "ready" | "missing" | "broken";
  installedVersion?: string | null;
  bundledVersion?: string | null;
  previousVersion?: string | null;
  activePath: string;
  launcherPath: string;
  canRollback: boolean;
  detail: string;
}

interface CodexCredentialProfile {
  workspaceId: number;
  workspaceName: string;
  projectId: number;
  projectName: string | null;
  model: string;
  activatedAtEpochSeconds: number;
}

interface CodexWorkspaceOption {
  workspaceId: number;
  name: string;
}

interface CodexProjectOption {
  projectId: number;
  name: string;
}

interface CodexCredentialManagerState {
  codexConfigured: boolean;
  credentialStatus: "verified" | "unverified" | "not_configured" | "invalid" | "invalid_context";
  activeProfile: CodexCredentialProfile | null;
  profiles: CodexCredentialProfile[];
  workspaces: CodexWorkspaceOption[];
  discoveryWarning: string | null;
  sharedAuthPath: string;
  codexAuthPath: string;
  codexConfigPath: string;
}

interface CodexCredentialSwitchResult {
  state: CodexCredentialManagerState;
  codexRestarted: boolean;
  restartMessage: string;
}
const SHELL_SCHEMA = {
  type: "object",
  required: ["command"],
  properties: {
    command: {
      description:
        'Command argv array for direct execution. On Windows, run shell built-ins or PATH lookup through cmd /C, for example ["cmd", "/C", "where", "wechat-decrypt"].',
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
  const [savedServiceSignatures, setSavedServiceSignatures] = useState<string[]>([]);
  const [runtime, setRuntime] = useState<RuntimeSnapshot | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [logServiceFilter, setLogServiceFilter] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [runtimeConflict, setRuntimeConflict] = useState<RuntimeLockConflict | null>(null);
  const [browserAuth, setBrowserAuth] = useState<BrowserAuthStartResponse | null>(null);
  const [appVersion, setAppVersion] = useState<AppVersionInfo | null>(null);
  const [appUpdate, setAppUpdate] = useState<AppUpdateStatus | null>(null);
  const [desktopPermissions, setDesktopPermissions] = useState<DesktopPermissionStatus | null>(null);
  const [registeredServiceStatuses, setRegisteredServiceStatuses] = useState<RegisteredServiceStatus[]>([]);
  const [connectorApps, setConnectorApps] = useState<ConnectorSummary[]>([]);
  const [marketConnectors, setMarketConnectors] = useState<MarketConnector[]>([]);
  const [baijimuCli, setBaijimuCli] = useState<ManagedToolStatus | null>(null);
  const [codexCredentialManager, setCodexCredentialManager] = useState<CodexCredentialManagerState | null>(null);
  const [codexCredentialError, setCodexCredentialError] = useState("");
  const [codexCredentialBusy, setCodexCredentialBusy] = useState(false);
  const [codexWorkspaceId, setCodexWorkspaceId] = useState("");
  const [codexProjectId, setCodexProjectId] = useState("");
  const [codexProjectName, setCodexProjectName] = useState("");
  const [codexProjects, setCodexProjects] = useState<CodexProjectOption[]>([]);
  const [codexProjectsBusy, setCodexProjectsBusy] = useState(false);
  const [codexProjectsError, setCodexProjectsError] = useState("");
  const [connectorUpdateStatuses, setConnectorUpdateStatuses] = useState<Record<string, ConnectorAppUpdateStatus>>({});
  const [appUpdateCheckState, setAppUpdateCheckState] = useState<AppUpdateCheckState>("checking");
  const [appUpdateError, setAppUpdateError] = useState<string | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [appUpdateProgress, setAppUpdateProgress] = useState<AppUpdateProgress | null>(null);
  const [serviceStartBusy, setServiceStartBusy] = useState<string | null>(null);
  const [connectorBusy, setConnectorBusy] = useState<string | null>(null);
  const [localAppLifecycleOverrides, setLocalAppLifecycleOverrides] = useState<
    Record<string, LocalAppLifecycleOverride>
  >({});
  const [connectorUpdateBusy, setConnectorUpdateBusy] = useState<string | null>(null);
  const [managedToolBusy, setManagedToolBusy] = useState(false);
  const [serviceNotices, setServiceNotices] = useState<Record<number, string>>({});
  const [serviceJsonDrafts, setServiceJsonDrafts] = useState<Record<number, string>>({});
  const [serviceJsonErrors, setServiceJsonErrors] = useState<Record<number, string>>({});
  const [capabilityTestDrafts, setCapabilityTestDrafts] = useState<Record<string, string>>({});
  const [capabilityTestResults, setCapabilityTestResults] = useState<Record<string, CapabilityTestState>>({});
  const [capabilityTestBusy, setCapabilityTestBusy] = useState<string | null>(null);
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
  const [selectedLocalAppId, setSelectedLocalAppId] = useState<string | null>(null);
  const [activeLocalAppDetailTab, setActiveLocalAppDetailTab] = useState<LocalAppDetailTab>("overview");
  const [installPanelOpen, setInstallPanelOpen] = useState(false);
  const [installSourceMode, setInstallSourceMode] = useState<InstallSourceMode>("choose");
  const [selectedMarketAppId, setSelectedMarketAppId] = useState("");
  const [installSource, setInstallSource] = useState("");
  const [installBusy, setInstallBusy] = useState(false);

  useEffect(() => {
    void refreshAll();
  }, []);

  useEffect(() => {
    void loadAppVersion();
    void checkAppUpdate();
  }, []);

  useEffect(() => {
    void refreshDesktopPermissions();
  }, []);

  useEffect(() => {
    void refreshRegisteredServiceStatuses();
  }, []);

  useEffect(() => {
    void refreshConnectorApps();
  }, []);

  useEffect(() => {
    void refreshCodexCredentialManager();
  }, []);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<AppUpdateProgress>("app-update-progress", (event) => {
      if (active) {
        setAppUpdateProgress(event.payload);
      }
    }).then((dispose) => {
      if (active) {
        unlisten = dispose;
      } else {
        dispose();
      }
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!message) {
      return;
    }
    const timer = window.setTimeout(() => setMessage(""), 4500);
    return () => window.clearTimeout(timer);
  }, [message]);

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
    const timer = window.setInterval(() => {
      void refreshRegisteredServiceStatuses();
    }, 5000);
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

  const installableMarketConnectors = useMemo(
    () => marketConnectors.filter((app) => app.applicationType !== "managed_tool"),
    [marketConnectors]
  );

  useEffect(() => {
    setSelectedMarketAppId((current) => {
      if (current && installableMarketConnectors.some((app) => app.id === current)) {
        return current;
      }
      return installableMarketConnectors[0]?.id ?? "";
    });
  }, [installableMarketConnectors]);

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
    if (config && needsBrowserAuthorization(config)) {
      return "未授权";
    }
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
  }, [config, runtime]);
  const needsAuthorization = config ? needsBrowserAuthorization(config) : false;
  const startActionLocked =
    !needsAuthorization &&
    (runtime?.status === "starting" ||
      runtime?.status === "connecting" ||
      runtime?.status === "backoff" ||
      runtime?.status === "stopping");
  const runtimeCanStop = Boolean(
    runtime &&
      !needsAuthorization &&
      runtime.status !== "stopped" &&
      runtime.status !== "stopping"
  );
  const startActionLabel = needsAuthorization
    ? browserAuth
      ? "授权中"
      : "去授权"
    : !runtime
      ? "启动"
      : startActionLocked
        ? statusLabel
        : runtime.status !== "stopped"
          ? "重启"
          : "启动";

  const latestLog = logs.length > 0 ? logs[logs.length - 1] : null;
  const logServiceOptions = useMemo(() => {
    const names = new Set<string>();
    config?.services.forEach((service) => {
      if (service.name.trim()) {
        names.add(service.name.trim());
      }
    });
    logs.forEach((entry) => {
      if (entry.service?.trim()) {
        names.add(entry.service.trim());
      }
    });
    return Array.from(names).sort((left, right) => left.localeCompare(right));
  }, [config, logs]);
  const filteredLogs = useMemo(
    () => logs.filter((entry) => !logServiceFilter || entry.service === logServiceFilter),
    [logServiceFilter, logs]
  );
  const exposedCapabilityCount =
    config?.services.reduce(
      (count, service) =>
        count +
        (service.enabled
          ? service.methods.filter((method) => method.enabled).length +
            service.events.filter((event) => event.enabled).length
          : 0),
      0
    ) ?? 0;
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
  const localApps = useMemo<LocalAppItem[]>(() => {
    if (!config) {
      return [];
    }
    const connectorServiceNames = new Set<string>();
    const apps: LocalAppItem[] = connectorApps.map((connector) => {
      const serviceIndexes = connector.serviceNames
        .map((serviceName) => config.services.findIndex((service) => service.name === serviceName))
        .filter((index) => index >= 0);
      connector.serviceNames.forEach((serviceName) => connectorServiceNames.add(serviceName));
      return {
        id: `connector:${connector.id}`,
        name: connector.name,
        description: `版本 ${connector.version} · ${connector.serviceNames.length} 项能力组`,
        kind: "connector",
        serviceIndexes,
        connector,
        codexAccountManagement: connector.id === CODEX_CONNECTOR_ID
      };
    });

    if (baijimuCli) {
      apps.push({
        id: `managed-tool:${baijimuCli.id}`,
        name: baijimuCli.name,
        description: baijimuCli.description,
        kind: "managed_tool",
        serviceIndexes: [],
        managedTool: baijimuCli
      });
    }

    config.services.forEach((service, serviceIndex) => {
      if (connectorServiceNames.has(service.name)) {
        return;
      }
      if (isComputerService(service)) {
        apps.push({
          id: "built-in:desktop-control",
          name: "桌面控制",
          description: "截图、点击、输入、拖拽和按键能力。",
          kind: "built_in",
          serviceIndexes: [serviceIndex]
        });
        return;
      }
      if (isShellService(service)) {
        apps.push({
          id: "built-in:shell",
          name: "Shell",
          description: "受控执行本机命令。",
          kind: "built_in",
          serviceIndexes: [serviceIndex]
        });
        return;
      }
      apps.push({
        id: `custom:${service.name}:${serviceIndex}`,
        name: service.name || "未命名应用",
        description: service.description || "开发者自定义本地应用。",
        kind: "custom",
        serviceIndexes: [serviceIndex]
      });
    });
    return apps;
  }, [config, connectorApps, baijimuCli]);
  const selectedLocalApp =
    selectedLocalAppId == null ? null : localApps.find((app) => app.id === selectedLocalAppId) ?? null;
  const enabledLocalAppCount = localApps.filter(
    (app) =>
      app.managedTool?.state === "ready" ||
      app.serviceIndexes.some((serviceIndex) => config?.services[serviceIndex]?.enabled)
  ).length;

  useEffect(() => {
    setSelectedLocalAppId((current) =>
      current && localApps.some((app) => app.id === current) ? current : null
    );
  }, [localApps]);

  useEffect(() => {
    if (selectedLocalApp?.serviceIndexes.length) {
      setExpandedServiceIndex(selectedLocalApp.serviceIndexes[0]);
    }
    setActiveLocalAppDetailTab("overview");
  }, [selectedLocalApp?.id]);
  const appVersionLabel =
    appVersion?.currentVersion ?? appUpdate?.currentVersion ?? "检查中";
  const forceUpdateRequired = appUpdate?.forceUpdateRequired === true;
  const appUpdateStatusLabel = appUpdate
    ? forceUpdateRequired
      ? `必须升级到 ${appUpdate.latestVersion ?? appUpdate.minimumSupportedVersion ?? "最新版本"}`
      : appUpdate.updateAvailable
      ? `可升级到 ${appUpdate.latestVersion ?? "-"}`
      : "已是最新版本"
    : appUpdateCheckState === "error"
      ? appUpdateError || "检查失败"
      : "检查中";
  const appUpdateTone =
    forceUpdateRequired || appUpdate?.updateAvailable || appUpdateCheckState === "error" ? "danger" : "normal";
  const appUpdateProgressPercent = calculateAppUpdateProgressPercent(appUpdateProgress);
  const hasDesktopPermissionGap =
    enabledComputerMethodCount > 0 &&
    desktopPermissions != null &&
    ((!desktopPermissions.accessibilityGranted && desktopPermissions.accessibilitySupported) ||
      (!desktopPermissions.screenRecordingGranted && desktopPermissions.screenRecordingSupported));

  function buildMethodEditorKey(serviceIndex: number, methodIndex: number) {
    return `${serviceIndex}:${methodIndex}`;
  }

  function buildCapabilityTestKey(serviceIndex: number, methodIndex: number) {
    return `${serviceIndex}:${methodIndex}`;
  }

  function isMethodAdvancedOpen(serviceIndex: number, methodIndex: number) {
    return expandedMethodAdvancedKey === buildMethodEditorKey(serviceIndex, methodIndex);
  }

  function toggleMethodAdvanced(serviceIndex: number, methodIndex: number) {
    const key = buildMethodEditorKey(serviceIndex, methodIndex);
    setExpandedMethodAdvancedKey((current) => (current === key ? null : key));
  }

  function openLocalAppCapabilityConfig(serviceIndex: number, methodIndex?: number) {
    if (methodIndex != null) {
      toggleMethodAdvanced(serviceIndex, methodIndex);
    }
    setExpandedServiceIndex(serviceIndex);
  }

  function applyConfigDocument(document: ConfigDocument) {
    const uiConfig = toUiConfig(document.config);
    setConfigPath(document.config_path);
    setManifestPreview(document.manifest_preview);
    setConfig(uiConfig);
    setSavedServiceSignatures(uiConfig.services.map(serviceSignature));
    setRuntime(document.runtime);
    setRuntimeConflict(null);
    setServiceNotices({});
    setServiceJsonDrafts({});
    setServiceJsonErrors({});
    setCapabilityTestDrafts({});
    setCapabilityTestResults({});
  }

  function applySavedServiceDocument(document: ConfigDocument, serviceIndex: number) {
    const uiConfig = toUiConfig(document.config);
    const savedService = uiConfig.services[serviceIndex];
    setConfigPath(document.config_path);
    setManifestPreview(document.manifest_preview);
    setRuntime(document.runtime);
    if (!savedService) {
      applyConfigDocument(document);
      return;
    }
    setConfig((current) => {
      if (!current) {
        return uiConfig;
      }
      const services = [...current.services];
      services[serviceIndex] = savedService;
      return { ...current, services };
    });
    setSavedServiceSignatures((current) => {
      const signatures = [...current];
      signatures[serviceIndex] = serviceSignature(savedService);
      return signatures;
    });
    setServiceNotices((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
    setServiceJsonDrafts((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
    setServiceJsonErrors((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
    setCapabilityTestDrafts({});
    setCapabilityTestResults({});
  }

  function applyDeletedServiceDocument(document: ConfigDocument, serviceIndex: number) {
    const uiConfig = toUiConfig(document.config);
    setConfigPath(document.config_path);
    setManifestPreview(document.manifest_preview);
    setRuntime(document.runtime);
    setConfig((current) => {
      if (!current) {
        return uiConfig;
      }
      return {
        ...current,
        services: current.services.filter((_, index) => index !== serviceIndex)
      };
    });
    setSavedServiceSignatures((current) => current.filter((_, index) => index !== serviceIndex));
    setServiceNotices((current) => reindexRecordAfterDelete(current, serviceIndex));
    setServiceJsonDrafts((current) => reindexRecordAfterDelete(current, serviceIndex));
    setServiceJsonErrors((current) => reindexRecordAfterDelete(current, serviceIndex));
    setCapabilityTestDrafts({});
    setCapabilityTestResults({});
  }

  function formatApplyMessage(base: string, snapshot: RuntimeSnapshot) {
    return snapshot.status === "stopped"
      ? `${base}，Agent 未运行，启动后生效`
      : `${base}，已应用到正在运行的 Agent`;
  }

  function handleCommandError(err: unknown) {
    const conflict = readRuntimeConflict(err);
    if (conflict) {
      setRuntimeConflict(conflict);
      setError("");
      setActivePage("overview");
      return;
    }
    setError(readError(err));
  }

  async function refreshAll() {
    try {
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigDocument>("load_config");
      applyConfigDocument(document);
      const latestLogs = await invoke<LogEntry[]>("list_logs", { limit: 200 });
      setLogs(latestLogs);
      await refreshMarketConnectorApps();
      await refreshConnectorApps();
      await refreshBaijimuCli();
      await refreshCodexCredentialManager();
      await refreshRegisteredServiceStatuses();
    } catch (err) {
      handleCommandError(err);
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

  async function refreshRegisteredServiceStatuses() {
    try {
      const statuses = await invoke<RegisteredServiceStatus[]>("registered_service_statuses");
      setRegisteredServiceStatuses(statuses);
    } catch (err) {
      console.warn("读取本地应用运行状态失败", err);
    }
  }

  async function refreshConnectorApps() {
    try {
      const apps = await invoke<ConnectorSummary[]>("list_connector_apps");
      setConnectorApps(apps);
    } catch (err) {
      console.warn("读取本地应用列表失败", err);
    }
  }

  async function refreshBaijimuCli() {
    try {
      setBaijimuCli(await invoke<ManagedToolStatus>("baijimu_cli_status"));
    } catch (err) {
      console.warn("读取 Baijimu CLI 托管状态失败", err);
      setBaijimuCli(null);
    }
  }

  async function refreshCodexCredentialManager() {
    try {
      setCodexCredentialError("");
      const state = await invoke<CodexCredentialManagerState>("invoke_connector_management", {
        id: CODEX_CONNECTOR_ID,
        operation: "credentialState",
        payload: null
      });
      setCodexCredentialManager(state);
      const preferredWorkspaceId =
        Number(codexWorkspaceId) || state.activeProfile?.workspaceId || state.workspaces[0]?.workspaceId || 0;
      if (preferredWorkspaceId > 0) {
        selectCodexWorkspace(state, preferredWorkspaceId);
      }
    } catch (err) {
      setCodexCredentialManager(null);
      setCodexCredentialError(readError(err));
    }
  }

  function selectCodexWorkspace(state: CodexCredentialManagerState | null, workspaceId: number) {
    setCodexWorkspaceId(workspaceId > 0 ? String(workspaceId) : "");
    const profile =
      state?.activeProfile?.workspaceId === workspaceId
        ? state.activeProfile
        : state?.profiles.find((item) => item.workspaceId === workspaceId);
    setCodexProjectId(profile ? String(profile.projectId) : "");
    setCodexProjectName(profile?.projectName ?? "");
    void refreshCodexProjects(workspaceId);
  }

  async function refreshCodexProjects(workspaceId: number) {
    if (!workspaceId) {
      setCodexProjects([]);
      return;
    }
    try {
      setCodexProjectsBusy(true);
      setCodexProjectsError("");
      const projects = await invoke<CodexProjectOption[]>("invoke_connector_management", {
        id: CODEX_CONNECTOR_ID,
        operation: "listWorkspaceProjects",
        payload: { workspaceId }
      });
      setCodexProjects(projects);
    } catch (err) {
      setCodexProjects([]);
      setCodexProjectsError(`${readError(err)}；仍可直接填写项目 ID。`);
    } finally {
      setCodexProjectsBusy(false);
    }
  }

  async function switchCodexCredential(profile?: CodexCredentialProfile) {
    const workspaceId = profile?.workspaceId ?? Number(codexWorkspaceId);
    const projectId = profile?.projectId ?? Number(codexProjectId);
    const workspace = codexCredentialManager?.workspaces.find((item) => item.workspaceId === workspaceId);
    const selectedProject = codexProjects.find((item) => item.projectId === projectId);
    const workspaceName = profile?.workspaceName ?? workspace?.name ?? `工作区 ${workspaceId}`;
    const projectName = profile?.projectName ?? selectedProject?.name ?? (codexProjectName.trim() || null);
    if (!Number.isInteger(workspaceId) || workspaceId <= 0) {
      setCodexCredentialError("请选择要切换的工作区。");
      return;
    }
    if (!Number.isInteger(projectId) || projectId <= 0) {
      setCodexCredentialError("请输入有效的项目 ID；Codex 调用必须有明确的项目归属。");
      return;
    }
    const confirmed = window.confirm(
      `将为“${workspaceName}”的项目 ${projectName || projectId} 重新签发 LLM credential，并重启 Codex。继续吗？`
    );
    if (!confirmed) {
      return;
    }
    try {
      setCodexCredentialBusy(true);
      setCodexCredentialError("");
      setMessage("");
      const result = await invoke<CodexCredentialSwitchResult>("invoke_connector_management", {
        id: CODEX_CONNECTOR_ID,
        operation: "switchCredential",
        payload: {
          workspaceId,
          workspaceName,
          projectId,
          projectName,
          model: profile?.model ?? "gpt-5.6-sol"
        }
      });
      setCodexCredentialManager(result.state);
      selectCodexWorkspace(result.state, workspaceId);
      setMessage(`Codex 已切换到 ${workspaceName} / ${projectName || `项目 ${projectId}`}。${result.restartMessage}`);
    } catch (err) {
      setCodexCredentialError(readError(err));
    } finally {
      setCodexCredentialBusy(false);
    }
  }
  async function refreshMarketConnectorApps() {
    try {
      const apps = await invoke<MarketConnector[]>("list_market_connector_apps");
      setMarketConnectors(apps);
    } catch (err) {
      console.warn("读取本地应用市场失败", err);
      setMarketConnectors([]);
    }
  }

  async function startRegisteredService(serviceName: string): Promise<boolean> {
    try {
      setServiceStartBusy(serviceName);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const result = await invoke<StartRegisteredServiceResult>("start_registered_service", {
        service: serviceName
      });
      if (result.success) {
        const snapshot = await invoke<RuntimeSnapshot>("apply_saved_config_to_runtime");
        setRuntime(snapshot);
        setMessage(formatApplyMessage(`应用 ${serviceName} 的启动命令已执行`, snapshot));
        return true;
      } else {
        setError(
          `应用 ${serviceName} 启动命令执行失败` +
            (result.exitCode == null ? "" : `，退出码 ${result.exitCode}`) +
            (result.stderr.trim() ? `：${result.stderr.trim()}` : "")
        );
        return false;
      }
    } catch (err) {
      handleCommandError(err);
      return false;
    } finally {
      await refreshRegisteredServiceStatuses();
      setServiceStartBusy(null);
    }
  }

  async function stopRegisteredService(serviceName: string): Promise<boolean> {
    try {
      setServiceStartBusy(serviceName);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const result = await invoke<StartRegisteredServiceResult>("stop_registered_service", {
        service: serviceName
      });
      if (result.success) {
        setMessage(`应用 ${serviceName} 已停止`);
        return true;
      } else {
        setError(
          `应用 ${serviceName} 停止命令执行失败` +
            (result.exitCode == null ? "" : `，退出码 ${result.exitCode}`) +
            (result.stderr.trim() ? `：${result.stderr.trim()}` : "")
        );
        return false;
      }
    } catch (err) {
      handleCommandError(err);
      return false;
    } finally {
      await refreshRegisteredServiceStatuses();
      setServiceStartBusy(null);
    }
  }

  async function installLocalApp() {
    const selectedMarket = installableMarketConnectors.find((app) => app.id === selectedMarketAppId);
    const source =
      installSourceMode === "market" ? selectedMarket?.source ?? "" : installSource.trim();
    if (!source) {
      setError(installSourceMode === "market" ? "请选择要安装的应用" : "请输入本地目录或 Git 仓库地址");
      return;
    }
    try {
      setInstallBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConnectorAppInstallDocument>("install_connector_app", {
        source,
        replace: true,
        checksum: installSourceMode === "market" ? selectedMarket?.checksum ?? null : null,
        allowGit: installSourceMode === "custom"
      });
      applyConfigDocument(document.config);
      await refreshConnectorApps();
      await refreshRegisteredServiceStatuses();
      setSelectedLocalAppId(`connector:${document.install.connectorId}`);
      setActivePage("apps");
      setInstallPanelOpen(false);
      setMessage(`应用 ${document.install.name} ${document.install.version} 已安装`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setInstallBusy(false);
    }
  }

  function marketConnectorForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    if (app.kind !== "connector" || !app.connector) {
      return undefined;
    }
    return marketConnectors.find((marketApp) => marketApp.connectorId === app.connector?.id);
  }

  function marketManagedToolForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    if (app.kind !== "managed_tool" || !app.managedTool) {
      return undefined;
    }
    return marketConnectors.find(
      (marketApp) =>
        marketApp.applicationType === "managed_tool" && marketApp.connectorId === app.managedTool?.id
    );
  }

  async function upgradeManagedTool(app: LocalAppItem) {
    const marketApp = marketManagedToolForLocalApp(app);
    if (!app.managedTool || !marketApp) {
      setError(`工具 ${app.name} 没有关联的官方更新源`);
      return;
    }
    if (!marketApp.source || !marketApp.checksum) {
      setError(`工具 ${app.name} 的市场版本缺少下载地址或 SHA-256 校验值`);
      return;
    }
    try {
      setManagedToolBusy(true);
      setMessage("");
      setError("");
      const status = await invoke<ManagedToolStatus>("install_baijimu_cli_update", {
        version: marketApp.version,
        source: marketApp.source,
        checksum: marketApp.checksum,
        archivePath: marketApp.archivePath ?? null
      });
      setBaijimuCli(status);
      setMessage(`${status.name} 已升级到 ${status.installedVersion}`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setManagedToolBusy(false);
    }
  }

  async function rollbackManagedTool(app: LocalAppItem) {
    if (!app.managedTool?.canRollback) {
      setError(`工具 ${app.name} 没有可回滚的历史版本`);
      return;
    }
    try {
      setManagedToolBusy(true);
      setMessage("");
      setError("");
      const status = await invoke<ManagedToolStatus>("rollback_baijimu_cli");
      setBaijimuCli(status);
      setMessage(`${status.name} 已回滚到 ${status.installedVersion}`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setManagedToolBusy(false);
    }
  }

  function connectorSyncSource(app: LocalAppItem): string {
    const connector = app.connector;
    if (!connector) {
      return "";
    }
    return (connector.sourceReference ?? connector.sourcePath ?? "").trim();
  }

  function connectorSourceKind(app: LocalAppItem, marketApp?: MarketConnector): string {
    if (marketApp) {
      return "市场应用";
    }
    const source = connectorSyncSource(app);
    return isGitSourceText(source) ? "Git 仓库" : "本地目录";
  }

  function isGitSourceText(source: string): boolean {
    const value = source.trim();
    return (
      value.startsWith("https://") ||
      value.startsWith("http://") ||
      value.startsWith("git@") ||
      value.endsWith(".git") ||
      value.includes(".git#")
    );
  }

  async function checkLocalAppUpdate(app: LocalAppItem, showLatestMessage = true) {
    const marketApp = marketConnectorForLocalApp(app);
    if (!app.connector || !marketApp) {
      setError(`应用 ${app.name} 没有关联的市场更新源`);
      return null;
    }
    try {
      setConnectorUpdateBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const status = await invoke<ConnectorAppUpdateStatus>("check_connector_app_update", {
        id: app.connector.id,
        source: marketApp.source,
        checksum: marketApp.checksum ?? null,
        allowGit: false
      });
      setConnectorUpdateStatuses((current) => ({
        ...current,
        [app.connector!.id]: status
      }));
      if (showLatestMessage) {
        setMessage(
          status.updateAvailable
            ? `发现 ${status.name} ${status.latestVersion}，当前版本 ${status.currentVersion}`
            : `${status.name} 当前已经是最新版本 ${status.currentVersion}`
        );
      }
      return status;
    } catch (err) {
      handleCommandError(err);
      return null;
    } finally {
      setConnectorUpdateBusy(null);
    }
  }

  async function upgradeLocalApp(app: LocalAppItem) {
    const marketApp = marketConnectorForLocalApp(app);
    if (!app.connector || !marketApp) {
      setError(`应用 ${app.name} 没有关联的市场更新源`);
      return;
    }
    try {
      setConnectorUpdateBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConnectorAppInstallDocument>("install_connector_app", {
        source: marketApp.source,
        replace: true,
        checksum: marketApp.checksum ?? null,
        allowGit: false
      });
      applyConfigDocument(document.config);
      await refreshConnectorApps();
      await refreshRegisteredServiceStatuses();
      setConnectorUpdateStatuses((current) => ({
        ...current,
        [document.install.connectorId]: {
          connectorId: document.install.connectorId,
          name: document.install.name,
          currentVersion: document.install.version,
          latestVersion: document.install.version,
          updateAvailable: false,
          source: marketApp.source
        }
      }));
      setSelectedLocalAppId(`connector:${document.install.connectorId}`);
      setMessage(`应用 ${document.install.name} 已升级到 ${document.install.version}`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setConnectorUpdateBusy(null);
    }
  }

  async function syncLocalApp(app: LocalAppItem) {
    if (!app.connector) {
      setError(`应用 ${app.name} 不是可同步安装应用`);
      return;
    }
    const source = connectorSyncSource(app);
    if (!source) {
      setError(`应用 ${app.name} 没有记录安装来源，无法重新同步`);
      return;
    }
    try {
      setConnectorUpdateBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConnectorAppInstallDocument>("install_connector_app", {
        source,
        replace: true,
        checksum: null,
        allowGit: true
      });
      applyConfigDocument(document.config);
      await refreshConnectorApps();
      await refreshRegisteredServiceStatuses();
      setConnectorUpdateStatuses((current) => {
        const next = { ...current };
        delete next[document.install.connectorId];
        return next;
      });
      setSelectedLocalAppId(`connector:${document.install.connectorId}`);
      setMessage(`应用 ${document.install.name} 已重新同步到 ${document.install.version}`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setConnectorUpdateBusy(null);
    }
  }

  async function startLocalApp(app: LocalAppItem) {
    if (!hasLocalAppStartCommand(app)) {
      return;
    }
    try {
      setConnectorBusy(app.id);
      setLocalAppLifecycleOverride(app.id, {
        state: "starting",
        detail: "正在执行应用启动命令"
      });
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      if (app.kind === "connector" && app.connector) {
        const result = await invoke<ConnectorStartResult>("start_connector_app", {
          id: app.connector.id
        });
        const failed = result.services.filter((service) => service.exitCode !== 0);
        await refreshRegisteredServiceStatuses();
        if (failed.length > 0) {
          setLocalAppLifecycleOverride(app.id, {
            state: "start_failed",
            detail: formatConnectorServiceFailures(failed)
          });
          setError(
            `应用 ${app.name} 启动失败：` +
              formatConnectorServiceFailures(failed)
          );
        } else {
          const snapshot = await invoke<RuntimeSnapshot>("apply_saved_config_to_runtime");
          setRuntime(snapshot);
          setLocalAppLifecycleOverride(app.id, {
            state: "running",
            detail: "启动命令已执行"
          });
          setMessage(formatApplyMessage(`应用 ${app.name} 已启动`, snapshot));
        }
        return;
      }

      const startableService = app.serviceIndexes
        .map((index) => config?.services[index])
        .find((service): service is UiServiceConfig => Boolean(service?.start_command));
      if (!startableService) {
        setError(`应用 ${app.name} 没有配置启动命令`);
        return;
      }
      const started = await startRegisteredService(startableService.name);
      setLocalAppLifecycleOverride(app.id, {
        state: started ? "running" : "start_failed",
        detail: started ? "启动命令已执行" : "启动命令执行失败"
      });
    } catch (err) {
      setLocalAppLifecycleOverride(app.id, {
        state: "start_failed",
        detail: readError(err)
      });
      handleCommandError(err);
    } finally {
      setConnectorBusy(null);
    }
  }

  async function stopLocalApp(app: LocalAppItem) {
    if (!hasLocalAppStopCommand(app)) {
      return;
    }
    try {
      setConnectorBusy(app.id);
      setLocalAppLifecycleOverride(app.id, {
        state: "stopping",
        detail: "正在执行应用停止命令"
      });
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      if (app.kind === "connector" && app.connector) {
        const result = await invoke<ConnectorStartResult>("stop_connector_app", {
          id: app.connector.id
        });
        const failed = result.services.filter((service) => service.exitCode !== 0);
        await refreshRegisteredServiceStatuses();
        if (failed.length > 0) {
          setLocalAppLifecycleOverride(app.id, {
            state: "running",
            detail: `停止失败：${formatConnectorServiceFailures(failed)}`
          });
          setError(
            `应用 ${app.name} 停止失败：` +
              formatConnectorServiceFailures(failed)
          );
        } else {
          setLocalAppLifecycleOverride(app.id, {
            state: "stopped",
            detail: "停止命令已执行"
          });
          setMessage(`应用 ${app.name} 已停止`);
        }
        return;
      }

      const stoppableService = app.serviceIndexes
        .map((index) => config?.services[index])
        .find((service): service is UiServiceConfig => Boolean(service?.stop_command));
      if (!stoppableService) {
        setError(`应用 ${app.name} 没有配置停止命令`);
        return;
      }
      const stopped = await stopRegisteredService(stoppableService.name);
      if (stopped) {
        setLocalAppLifecycleOverride(app.id, {
          state: "stopped",
          detail: "停止命令已执行"
        });
      }
    } catch (err) {
      setLocalAppLifecycleOverride(app.id, {
        state: "running",
        detail: `停止失败：${readError(err)}`
      });
      handleCommandError(err);
    } finally {
      setConnectorBusy(null);
    }
  }

  async function testCapability(serviceIndex: number, methodIndex: number) {
    if (!config?.services[serviceIndex]?.methods[methodIndex]) {
      return;
    }
    const service = config.services[serviceIndex];
    const method = service.methods[methodIndex];
    const testKey = buildCapabilityTestKey(serviceIndex, methodIndex);
    const draft = capabilityTestDrafts[testKey] ?? defaultCapabilityArgumentsText(method);
    try {
      setCapabilityTestBusy(testKey);
      setError("");
      const argumentsValue = parseJson(draft);
      const result = await invoke<CapabilityInvokeResult>("test_capability", {
        config: fromUiConfig(config),
        service: service.name,
        method: method.name,
        arguments: argumentsValue
      });
      setCapabilityTestResults((current) => ({
        ...current,
        [testKey]: {
          status: result.success ? "success" : "error",
          result,
          message: result.success ? "测试通过" : result.error?.message ?? "测试失败"
        }
      }));
      await refreshRuntime();
      await refreshRegisteredServiceStatuses();
    } catch (err) {
      setCapabilityTestResults((current) => ({
        ...current,
        [testKey]: {
          status: "error",
          message: readError(err)
        }
      }));
    } finally {
      setCapabilityTestBusy(null);
    }
  }

  async function uninstallLocalApp(app: LocalAppItem) {
    if (app.kind !== "connector" || !app.connector) {
      return;
    }
    try {
      setConnectorBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigDocument>("uninstall_connector_app", {
        id: app.connector.id
      });
      applyConfigDocument(document);
      await refreshConnectorApps();
      await refreshRegisteredServiceStatuses();
      setSelectedLocalAppId(null);
      setMessage(`应用 ${app.name} 已卸载`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setConnectorBusy(null);
    }
  }

  async function checkAppUpdate(showLatestMessage = false) {
    setAppUpdateCheckState("checking");
    setAppUpdateError(null);
    try {
      const status = await invoke<AppUpdateStatus>("check_app_update");
      setAppUpdate(status);
      setAppUpdateCheckState("ready");
      if (showLatestMessage) {
        setMessage(
          status.forceUpdateRequired
            ? `当前版本 ${status.currentVersion} 已停止支持，需要升级到 ${status.latestVersion ?? status.minimumSupportedVersion ?? "最新版本"} 后继续使用。`
            : status.updateAvailable
            ? status.autoDownloadAvailable
              ? `发现新版本 ${status.latestVersion}，可以下载后自动退出、替换并重启。`
              : `发现新版本 ${status.latestVersion}，但当前平台需要跳转发布页手工下载。`
            : `当前已经是最新版本 ${status.currentVersion}`
        );
      }
    } catch (err) {
      const message = readError(err);
      setAppUpdateCheckState("error");
      setAppUpdateError(message);
      if (showLatestMessage) {
        setError(message);
      } else {
        console.warn("自动检查更新失败", err);
      }
    }
  }

  async function loadAppVersion() {
    try {
      setAppVersion(await invoke<AppVersionInfo>("app_version"));
    } catch (err) {
      console.warn("读取本地应用版本失败", err);
    }
  }

  function renderAppUpdateProgress() {
    if (!updateBusy && appUpdateProgress?.phase !== "ready_to_install") {
      return null;
    }
    const progress = appUpdateProgress;
    const label = progress?.message ?? "正在准备更新";
    const detail = progress
      ? formatAppUpdateProgressDetail(progress)
      : "正在连接更新服务，请稍候。";

    return (
      <div className="app-update-progress" role="status" aria-live="polite">
        <div className="app-update-progress-head">
          <strong>{label}</strong>
          <span>{appUpdateProgressPercent == null ? "等待响应" : `${appUpdateProgressPercent}%`}</span>
        </div>
        <div className="app-update-progress-track" aria-hidden="true">
          <div
            className={`app-update-progress-bar ${appUpdateProgressPercent == null ? "indeterminate" : ""}`}
            style={appUpdateProgressPercent == null ? undefined : { width: `${appUpdateProgressPercent}%` }}
          />
        </div>
        <p>{detail}</p>
      </div>
    );
  }

  async function installAppUpdate() {
    try {
      setUpdateBusy(true);
      setAppUpdateProgress({
        phase: "checking",
        message: "正在获取最新版本信息",
        version: appUpdate?.latestVersion ?? null,
        assetName: appUpdate?.assetName ?? null,
        downloadedBytes: null,
        totalBytes: null,
        downloadedPath: null
      });
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const result = await invoke<AppUpdateInstallResult>("install_app_update");
      if (result.status === "up_to_date") {
        setMessage(`当前已经是最新版本 ${result.version}`);
        return;
      }
      setMessage(
        result.downloadedPath
          ? `更新包 ${result.assetName ?? ""} 已下载到 ${result.downloadedPath}，应用即将退出并自动完成替换。`
          : `更新包 ${result.assetName ?? ""} 已下载，应用即将退出并自动完成替换。`
      );
    } catch (err) {
      setAppUpdateProgress(null);
      handleCommandError(err);
    } finally {
      setUpdateBusy(false);
    }
  }

  async function saveConfig() {
    if (!config) {
      return;
    }
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigDocument>("save_config", {
        config: fromUiConfig(config)
      });
      applyConfigDocument(document);
      setMessage("配置已保存");
    } catch (err) {
      handleCommandError(err);
    } finally {
      setBusy(false);
    }
  }

  async function saveService(serviceIndex: number, applyToRuntime = false) {
    if (!config?.services[serviceIndex]) {
      return;
    }
    const service = config.services[serviceIndex];
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigDocument>("save_service", {
        serviceIndex,
        service: fromUiService(service),
        applyToRuntime
      });
      applySavedServiceDocument(document, serviceIndex);
      setExpandedServiceIndex(Math.min(serviceIndex, document.config.services.length - 1));
      const serviceName = service.name.trim() || "未命名应用";
      setServiceNotices((current) => ({
        ...current,
        [serviceIndex]: applyToRuntime
          ? formatApplyMessage(`应用 ${serviceName} 已保存`, document.runtime)
          : `应用 ${serviceName} 已保存`
      }));
      await refreshRegisteredServiceStatuses();
    } catch (err) {
      handleCommandError(err);
    } finally {
      setBusy(false);
    }
  }

  async function deleteSavedService(serviceIndex: number, applyToRuntime = false) {
    if (!config?.services[serviceIndex]) {
      return;
    }
    const serviceName = config.services[serviceIndex].name.trim() || "未命名应用";
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigDocument>("delete_service", {
        serviceIndex,
        applyToRuntime
      });
      applyDeletedServiceDocument(document, serviceIndex);
      setExpandedServiceIndex(document.config.services.length === 0 ? null : Math.max(0, serviceIndex - 1));
      setMessage(
        applyToRuntime
          ? formatApplyMessage(`应用 ${serviceName} 已删除`, document.runtime)
          : `应用 ${serviceName} 已删除`
      );
      await refreshRegisteredServiceStatuses();
    } catch (err) {
      handleCommandError(err);
    } finally {
      setBusy(false);
    }
  }

  async function startAgent() {
    if (!config) {
      return;
    }
    if (needsBrowserAuthorization(config)) {
      await beginBrowserAuth();
      return;
    }
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const snapshot = await invoke<RuntimeSnapshot>("start_agent", {
        config: fromUiConfig(config)
      });
      setSavedServiceSignatures(config.services.map(serviceSignature));
      setRuntime(snapshot);
      setMessage(formatStartAgentMessage(snapshot));
      await refreshRuntime();
    } catch (err) {
      const conflict = readRuntimeConflict(err);
      if (conflict) {
        setRuntimeConflict(conflict);
        setActivePage("overview");
      } else {
        setError(readError(err));
      }
    } finally {
      setBusy(false);
    }
  }

  async function stopConflictingRuntimeAndStart() {
    if (!runtimeConflict || !config) {
      return;
    }
    try {
      setBusy(true);
      setMessage("");
      setError("");
      await invoke("stop_conflicting_runtime", {
        lockPath: runtimeConflict.lock_path,
        pid: runtimeConflict.pid,
        agentId: runtimeConflict.agent_id,
        configPath: runtimeConflict.config_path
      });
      setRuntimeConflict(null);
      const snapshot = await invoke<RuntimeSnapshot>("start_agent", {
        config: fromUiConfig(config)
      });
      setSavedServiceSignatures(config.services.map(serviceSignature));
      setRuntime(snapshot);
      setMessage("已停止旧实例并重新启动 Agent");
      await refreshRuntime();
    } catch (err) {
      const conflict = readRuntimeConflict(err);
      if (conflict) {
        setRuntimeConflict(conflict);
      } else {
        setError(readError(err));
      }
    } finally {
      setBusy(false);
    }
  }

  async function stopAgent() {
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const snapshot = await invoke<RuntimeSnapshot>("stop_agent");
      setRuntime(snapshot);
      setMessage("Agent 已停止");
      await refreshRuntime();
    } catch (err) {
      handleCommandError(err);
    } finally {
      setBusy(false);
    }
  }

  async function resetExampleConfig() {
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigDocument>("reset_example_config");
      applyConfigDocument(document);
      setMessage("已恢复示例配置");
    } catch (err) {
      handleCommandError(err);
    } finally {
      setBusy(false);
    }
  }

  async function recoverInvalidConfig() {
    try {
      setBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const document = await invoke<ConfigRecoveryDocument>("recover_invalid_config");
      applyConfigDocument(document);
      setMessage(
        document.archived_path
          ? `已恢复默认配置，原配置已保留到 ${document.archived_path}`
          : "已创建默认配置"
      );
    } catch (err) {
      handleCommandError(err);
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

  async function openConsole() {
    if (!config) {
      return;
    }
    if (needsBrowserAuthorization(config)) {
      await beginBrowserAuth();
      return;
    }
    await openExternalUrl(buildConsoleUrl(config));
  }

  async function openExternalUrlInEdge(url: string) {
    try {
      await invoke("open_in_edge", { url });
    } catch (err) {
      setError(readError(err));
    }
  }

  async function copyText(text: string, label: string) {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        copyTextWithSelection(text);
      }
      setMessage(`${label}已复制`);
    } catch (err) {
      try {
        copyTextWithSelection(text);
        setMessage(`${label}已复制`);
      } catch {
        setError(readError(err));
      }
    }
  }

  function copyTextWithSelection(text: string) {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    const copied = document.execCommand("copy");
    document.body.removeChild(textarea);
    if (!copied) {
      throw new Error("复制失败");
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
      setMessage(`已打开浏览器授权页，用户码 ${session.userCode}。如果浏览器没有正常弹出，可复制授权链接手动打开。`);
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
        const uiConfig = toUiConfig(result.config);
        setConfig(uiConfig);
        setSavedServiceSignatures(uiConfig.services.map(serviceSignature));
        if (result.runtime) {
          setRuntime(result.runtime);
        }
        setBrowserAuth(null);
        setMessage("浏览器授权成功，Agent 已使用新凭证重启");
        return;
      }
      if (result.status === "denied" || result.status === "expired") {
        setBrowserAuth(null);
        setError(result.message);
      }
    } catch (err) {
      setBrowserAuth(null);
      handleCommandError(err);
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

  function updateServiceHealthCheck(
    serviceIndex: number,
    updater: (healthCheck: UiServiceHealthCheck) => UiServiceHealthCheck
  ) {
    updateService(serviceIndex, (service) => {
      const current = service.health_check ?? createServiceHealthCheck();
      return {
        ...service,
        health_check: updater(current)
      };
    });
  }

  function updateServiceStartCommand(
    serviceIndex: number,
    updater: (startCommand: UiServiceStartCommand) => UiServiceStartCommand
  ) {
    updateService(serviceIndex, (service) => {
      const current = service.start_command ?? createServiceStartCommand();
      return {
        ...service,
        start_command: updater(current)
      };
    });
  }

  function updateServiceStopCommand(
    serviceIndex: number,
    updater: (stopCommand: UiServiceStartCommand) => UiServiceStartCommand
  ) {
    updateService(serviceIndex, (service) => {
      const current = service.stop_command ?? createServiceStopCommand();
      return {
        ...service,
        stop_command: updater(current)
      };
    });
  }

  function updateServiceJsonDraft(serviceIndex: number, value: string) {
    setServiceJsonDrafts((current) => ({
      ...current,
      [serviceIndex]: value
    }));
    setServiceJsonErrors((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
  }

  function resetServiceJsonDraft(serviceIndex: number) {
    setServiceJsonDrafts((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
    setServiceJsonErrors((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
  }

  function applyServiceJsonDraft(serviceIndex: number) {
    const service = config?.services[serviceIndex];
    if (!service) {
      return;
    }
    try {
      const draft = serviceJsonDrafts[serviceIndex] ?? serviceCapabilitiesJson(service);
      const parsed = parseJson(draft) as Partial<ServiceCapabilitiesDocument>;
      if (!Array.isArray(parsed.methods)) {
        throw new Error("能力定义 JSON 必须包含 methods 数组");
      }
      if (parsed.events != null && !Array.isArray(parsed.events)) {
        throw new Error("能力定义 JSON 的 events 必须是数组");
      }
      const methods = parsed.methods.map(toUiMethod);
      const events = (parsed.events ?? []).map(toUiEvent);
      updateService(serviceIndex, (current) => ({
        ...current,
        methods,
        events
      }));
      resetServiceJsonDraft(serviceIndex);
    } catch (err) {
      setServiceJsonErrors((current) => ({
        ...current,
        [serviceIndex]: readError(err)
      }));
    }
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
                health_check: null,
                start_command: null,
                stop_command: null,
                methods: [createShellMethod()],
                events: []
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

  function addEvent(serviceIndex: number) {
    updateService(serviceIndex, (service) => ({
      ...service,
      events: [...service.events, createEvent()]
    }));
  }

  function removeMethod(serviceIndex: number, methodIndex: number) {
    updateService(serviceIndex, (service) => ({
      ...service,
      methods: service.methods.filter((_, index) => index !== methodIndex)
    }));
  }

  function removeEvent(serviceIndex: number, eventIndex: number) {
    updateService(serviceIndex, (service) => ({
      ...service,
      events: service.events.filter((_, index) => index !== eventIndex)
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

  function updateEvent(
    serviceIndex: number,
    eventIndex: number,
    updater: (event: UiEventConfig) => UiEventConfig
  ) {
    updateService(serviceIndex, (service) => ({
      ...service,
      events: service.events.map((event, index) =>
        index === eventIndex ? updater(event) : event
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
                    updateUpload("inline_limit_bytes", safeNumber(event.target.value, DEFAULT_INLINE_LIMIT_BYTES))
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
            <Field label="Node 路径" hint="留空时自动从 PATH、登录 shell 和桌面 App 内置 runtime 查找。">
              <input
                value={config.runtime.node_path ?? ""}
                onChange={(event) => updateRuntime("node_path", emptyToNull(event.target.value))}
                placeholder="/opt/homebrew/bin/node"
              />
            </Field>
            <Field label="Codex 命令路径" hint="留空时自动从 PATH 和登录 shell 查找。">
              <input
                value={config.runtime.codex_binary_path ?? ""}
                onChange={(event) =>
                  updateRuntime("codex_binary_path", emptyToNull(event.target.value))
                }
                placeholder="/opt/homebrew/bin/codex"
              />
            </Field>
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
            <Field label="文件日志">
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={config.runtime.log_file_enabled}
                  onChange={(event) => updateRuntime("log_file_enabled", event.target.checked)}
                />
                启用
              </label>
            </Field>
            <Field label="日志目录" hint="留空使用系统默认目录。">
              <input
                value={config.runtime.log_file_dir ?? ""}
                onChange={(event) => updateRuntime("log_file_dir", emptyToNull(event.target.value))}
                placeholder="C:\\ProgramData\\Baijimu\\BridgeAgent\\logs"
              />
            </Field>
            <Field label="单文件上限字节">
              <input
                type="number"
                min={1024}
                value={config.runtime.log_file_max_bytes}
                onChange={(event) =>
                  updateRuntime("log_file_max_bytes", safeNumber(event.target.value, 5 * 1024 * 1024))
                }
              />
            </Field>
            <Field label="轮转文件数">
              <input
                type="number"
                min={1}
                value={config.runtime.log_file_max_files}
                onChange={(event) =>
                  updateRuntime("log_file_max_files", safeNumber(event.target.value, 5))
                }
              />
            </Field>
            <Field label="本地事件入口">
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={config.runtime.event_server_enabled}
                  onChange={(event) => updateRuntime("event_server_enabled", event.target.checked)}
                />
                启用
              </label>
            </Field>
            <Field label="事件入口监听地址">
              <input
                value={config.runtime.event_server_bind}
                onChange={(event) => updateRuntime("event_server_bind", event.target.value)}
                placeholder="127.0.0.1:18081"
              />
            </Field>
            <Field label="事件入口 Token" hint="留空时只依赖本机监听地址。">
              <input
                type="password"
                value={config.runtime.event_server_token ?? ""}
                onChange={(event) => updateRuntime("event_server_token", emptyToNull(event.target.value))}
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
    const hasRuntimeConflict = Boolean(runtimeConflict);
    const hasRuntimeError = Boolean(runtime?.last_error) && !needsAuthorization;
    const hasAttention =
      needsAuthorization || hasRuntimeConflict || hasRuntimeError || hasDesktopPermissionGap;

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
              <button
                className="primary accent"
                onClick={() => void startAgent()}
                disabled={busy || startActionLocked}
              >
                {startActionLabel}
              </button>
              {runtimeCanStop ? (
                <button className="secondary" onClick={() => void stopAgent()} disabled={busy}>
                  停止
                </button>
              ) : null}
              <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
                重新授权
              </button>
              {!needsAuthorization ? (
                <button className="secondary" onClick={() => void openConsole()} disabled={busy}>
                  打开控制台
                </button>
              ) : null}
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

        <Card title="状态">
          <div className="status-detail-grid">
            <InfoRow label="连接状态" value={statusLabel} />
            <InfoRow
              label="Relay 注册"
              value={formatRelayRegistration(runtime)}
              tone={runtime?.relay_registered ? "normal" : "warning"}
            />
            <InfoRow label="工作区" value={config.platform.workspace_id || "未授权"} />
            <InfoRow
              label="最近错误"
              value={hasRuntimeError ? runtime?.last_error || "无" : "无"}
              tone={hasRuntimeError ? "danger" : "normal"}
            />
          </div>
        </Card>

        <Card title="需要处理">
          {hasAttention ? (
            <div className="attention-list">
              {needsAuthorization ? (
                <button className="attention-item" onClick={() => void beginBrowserAuth()} disabled={busy}>
                  <strong>需要浏览器授权</strong>
                  <span>完成授权后会自动写回工作区和连接凭证。</span>
                </button>
              ) : null}
              {hasDesktopPermissionGap ? (
                <button
                  className="attention-item"
                  onClick={() => {
                    setActivePage("apps");
                    const computerIndex = config.services.findIndex(isComputerService);
                    if (computerIndex >= 0) {
                      setExpandedServiceIndex(computerIndex);
                      setSelectedLocalAppId("built-in:desktop-control");
                    }
                  }}
                >
                  <strong>桌面控制需要权限</strong>
                  <span>打开桌面控制应用处理屏幕录制和辅助功能授权。</span>
                </button>
              ) : null}
              {runtimeConflict ? (
                <button className="attention-item" onClick={() => setActivePage("overview")}>
                  <strong>已有 Agent 实例占用运行锁</strong>
                  <span>PID {runtimeConflict.pid} 正在使用当前配置，可以停止旧实例后重新启动。</span>
                </button>
              ) : null}
              {hasRuntimeError ? (
                <button className="attention-item" onClick={() => setActivePage("diagnostics")}>
                  <strong>运行错误</strong>
                  <span>{runtime?.last_error}</span>
                </button>
              ) : null}
            </div>
          ) : (
            <div className="empty-state compact-empty">当前没有需要处理的问题。</div>
          )}
        </Card>

        <Card
          title="本地应用"
          action={
            <button className="secondary" onClick={() => setActivePage("apps")}>
              打开应用页
            </button>
          }
        >
          <div className="status-detail-grid">
            <InfoRow label="应用总数" value={String(localApps.length)} />
            <InfoRow label="已启用应用" value={String(enabledLocalAppCount)} />
            <InfoRow label="已开放能力" value={String(exposedCapabilityCount)} />
            <InfoRow label="最近日志" value={latestLog ? formatTime(latestLog.timestamp_ms) : "暂无"} />
          </div>
        </Card>
      </div>
    );
  }

  function renderRuntimeConflictPanel() {
    if (!runtimeConflict) {
      return null;
    }

    const processName = runtimeConflict.process.name || "未知进程";
    const processPath =
      runtimeConflict.process.executable_path || runtimeConflict.process.command_line || "无法读取进程路径";

    return (
      <div className="runtime-conflict-panel" role="alert">
        <div className="runtime-conflict-copy">
          <strong>百积木已经在运行</strong>
          <p>
            当前配置被 PID {runtimeConflict.pid} 占用。确认这是旧实例后，可以停止旧实例并重新启动。
          </p>
          <dl>
            <div>
              <dt>进程</dt>
              <dd>{processName}</dd>
            </div>
            <div>
              <dt>路径</dt>
              <dd>{processPath}</dd>
            </div>
            <div>
              <dt>锁文件</dt>
              <dd>{runtimeConflict.lock_path}</dd>
            </div>
          </dl>
        </div>
        <div className="runtime-conflict-actions">
          <button className="primary danger" onClick={() => void stopConflictingRuntimeAndStart()} disabled={busy}>
            停止旧实例并启动
          </button>
          <button className="secondary" onClick={() => setRuntimeConflict(null)} disabled={busy}>
            暂不处理
          </button>
        </div>
      </div>
    );
  }

  function renderBrowserAuthPanel() {
    if (!browserAuth) {
      return null;
    }

    return (
      <div className="browser-auth-panel" role="status">
        <div className="browser-auth-copy">
          <strong>等待浏览器授权</strong>
          <p>用户码 {browserAuth.userCode}</p>
          <input
            aria-label="授权链接"
            readOnly
            value={browserAuth.verificationUriComplete}
            onFocus={(event) => event.currentTarget.select()}
          />
        </div>
        <div className="browser-auth-actions">
          <button
            className="primary"
            onClick={() => void copyText(browserAuth.verificationUriComplete, "授权链接")}
          >
            复制链接
          </button>
          <button
            className="secondary"
            onClick={() => void openExternalUrlInEdge(browserAuth.verificationUriComplete)}
          >
            用 Edge 打开
          </button>
          <button
            className="secondary"
            onClick={() => void openExternalUrl(browserAuth.verificationUriComplete)}
          >
            默认浏览器打开
          </button>
        </div>
      </div>
    );
  }

  function serviceRuntimeView(service: UiServiceConfig): {
    status: RegisteredServiceStatus | undefined;
    statusClass: string | null;
    statusLabel: string | null;
    detail: string;
  } {
    const runtimeStatus = registeredServiceStatuses.find((status) => status.service === service.name);
    if (
      !service.health_check &&
      !service.start_command &&
      !service.stop_command &&
      (!runtimeStatus || runtimeStatus.status === "not_configured")
    ) {
      return {
        status: undefined,
        statusClass: null,
        statusLabel: null,
        detail: formatRegisteredServiceDetail(service, undefined)
      };
    }
    const statusIsStartableOnly =
      (runtimeStatus != null && !runtimeStatus.healthCheckConfigured && runtimeStatus.startCommandConfigured) ||
      (runtimeStatus == null && service.health_check == null && service.start_command != null);
    const runtimeStatusClass = statusIsStartableOnly ? "startable" : runtimeStatus?.status ?? "unknown";
    const runtimeStatusLabel = statusIsStartableOnly
      ? "可启动"
      : formatRegisteredServiceStatus(runtimeStatus?.status ?? "unknown");
    return {
      status: runtimeStatus,
      statusClass: runtimeStatusClass,
      statusLabel: runtimeStatusLabel,
      detail: formatRegisteredServiceDetail(service, runtimeStatus)
    };
  }

  function renderServiceRuntimePanel(service: UiServiceConfig, serviceIndex: number, isSystem: boolean) {
    const runtimeView = serviceRuntimeView(service);
    const serviceIsHealthy = runtimeView.status?.status === "healthy";
    const canStopService = service.stop_command != null;

    return (
      <div className="registered-service-runtime">
        <div className="registered-service-runtime-main">
          <div>
            <strong>运行</strong>
            <p>{runtimeView.detail}</p>
            {runtimeView.status ? (
              <span className="registered-service-runtime-meta">
                上次检查 {formatTime(runtimeView.status.checkedAtMs)}
              </span>
            ) : null}
          </div>
          {runtimeView.statusLabel && runtimeView.statusClass ? (
            <div className={`status-pill status-${runtimeView.statusClass}`}>
              {runtimeView.statusLabel}
            </div>
          ) : null}
        </div>
        <div className="registered-service-runtime-actions">
          {service.health_check ? (
            <button className="ghost" onClick={() => void refreshRegisteredServiceStatuses()}>
              检查状态
            </button>
          ) : !isSystem ? (
            <button
              className="ghost"
              onClick={() =>
                updateService(serviceIndex, (current) => ({
                  ...current,
                  health_check: createServiceHealthCheck()
                }))
              }
            >
              配置检查
            </button>
          ) : null}
          {serviceIsHealthy && canStopService ? (
            <button
              className="secondary danger"
              onClick={() => void stopRegisteredService(service.name)}
              disabled={serviceStartBusy != null}
            >
              {serviceStartBusy === service.name ? "停止中" : "停止应用"}
            </button>
          ) : service.start_command ? (
            <button
              className="secondary"
              onClick={() => void startRegisteredService(service.name)}
              disabled={serviceStartBusy != null}
            >
              {serviceStartBusy === service.name ? "启动中" : "启动应用"}
            </button>
          ) : !isSystem ? (
            <button
              className="secondary"
              onClick={() =>
                updateService(serviceIndex, (current) => ({
                  ...current,
                  start_command: createServiceStartCommand()
                }))
              }
            >
              配置启动
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  function renderServiceRuntimeConfig(service: UiServiceConfig, serviceIndex: number, isSystem: boolean) {
    if (isSystem) {
      return null;
    }

    return (
      <div className="runtime-config-section">
        <div className="method-advanced-head">
          <strong>运行配置</strong>
          <small>健康检查决定状态展示；启动/停止命令决定应用详情里的运行按钮。</small>
        </div>

        <div className="runtime-config-block">
          <div className="runtime-config-head">
            <div>
              <strong>健康检查</strong>
              <small>{service.health_check ? "已启用 HTTP 状态检查" : "未配置，无法自动判断是否可用"}</small>
            </div>
            {service.health_check ? (
              <button
                className="ghost danger"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    health_check: null
                  }))
                }
              >
                移除检查
              </button>
            ) : (
              <button
                className="secondary"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    health_check: createServiceHealthCheck()
                  }))
                }
              >
                添加检查
              </button>
            )}
          </div>
          {service.health_check ? (
            <div className="form-grid">
              <Field label="检查 URL" wide>
                <input
                  value={service.health_check.url}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      url: event.target.value
                    }))
                  }
                />
              </Field>
              <Field label="HTTP 方法">
                <input
                  value={service.health_check.http_method}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      http_method: event.target.value.toUpperCase()
                    }))
                  }
                />
              </Field>
              <Field label="期望状态码">
                <input
                  value={service.health_check.expect_status}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      expect_status: event.target.value
                    }))
                  }
                  placeholder="默认任意 2xx"
                />
              </Field>
              <Field label="超时秒数">
                <input
                  value={service.health_check.timeout_secs}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      timeout_secs: event.target.value
                    }))
                  }
                  placeholder="默认 3"
                />
              </Field>
              <Field label="响应包含">
                <input
                  value={service.health_check.body_contains}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      body_contains: event.target.value
                    }))
                  }
                  placeholder="可选"
                />
              </Field>
              <Field label="请求头" wide>
                <textarea
                  rows={4}
                  value={service.health_check.headers_text}
                  onChange={(event) =>
                    updateServiceHealthCheck(serviceIndex, (current) => ({
                      ...current,
                      headers_text: event.target.value
                    }))
                  }
                  placeholder={"Authorization: Bearer xxx\nX-App: local-service"}
                />
              </Field>
            </div>
          ) : null}
        </div>

        <div className="runtime-config-block">
          <div className="runtime-config-head">
            <div>
              <strong>启动命令</strong>
              <small>{service.start_command ? "已启用应用启动按钮" : "未配置，应用详情不会执行启动动作"}</small>
            </div>
            {service.start_command ? (
              <button
                className="ghost danger"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    start_command: null
                  }))
                }
              >
                移除启动
              </button>
            ) : (
              <button
                className="secondary"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    start_command: createServiceStartCommand()
                  }))
                }
              >
                添加启动
              </button>
            )}
          </div>
          {service.start_command ? (
            <div className="form-grid">
              <Field label="命令参数" hint="每行一个参数；第一行是可执行文件。" wide>
                <textarea
                  rows={5}
                  value={service.start_command.command_text}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      command_text: event.target.value
                    }))
                  }
                  placeholder={"npm\nrun\ndev"}
                />
              </Field>
              <Field label="工作目录">
                <input
                  value={service.start_command.cwd}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      cwd: event.target.value
                    }))
                  }
                  placeholder="留空继承 Agent 进程目录"
                />
              </Field>
              <Field label="超时秒数">
                <input
                  value={service.start_command.timeout_secs}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      timeout_secs: event.target.value
                    }))
                  }
                  placeholder="默认 20"
                />
              </Field>
              <Field label="环境变量" wide>
                <textarea
                  rows={4}
                  value={service.start_command.env_text}
                  onChange={(event) =>
                    updateServiceStartCommand(serviceIndex, (current) => ({
                      ...current,
                      env_text: event.target.value
                    }))
                  }
                  placeholder={"NODE_ENV: development\nPORT: 8081"}
                />
              </Field>
            </div>
          ) : null}
        </div>

        <div className="runtime-config-block">
          <div className="runtime-config-head">
            <div>
              <strong>停止命令</strong>
              <small>{service.stop_command ? "运行中会显示停止应用按钮" : "未配置，运行中无法从应用详情停止"}</small>
            </div>
            {service.stop_command ? (
              <button
                className="ghost danger"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    stop_command: null
                  }))
                }
              >
                移除停止
              </button>
            ) : (
              <button
                className="secondary"
                onClick={() =>
                  updateService(serviceIndex, (current) => ({
                    ...current,
                    stop_command: createServiceStopCommand()
                  }))
                }
              >
                添加停止
              </button>
            )}
          </div>
          {service.stop_command ? (
            <div className="form-grid">
              <Field label="命令参数" hint="每行一个参数；第一行是可执行文件。" wide>
                <textarea
                  rows={5}
                  value={service.stop_command.command_text}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      command_text: event.target.value
                    }))
                  }
                  placeholder={"npm\nrun\nstop"}
                />
              </Field>
              <Field label="工作目录">
                <input
                  value={service.stop_command.cwd}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      cwd: event.target.value
                    }))
                  }
                  placeholder="留空继承 Agent 进程目录"
                />
              </Field>
              <Field label="超时秒数">
                <input
                  value={service.stop_command.timeout_secs}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      timeout_secs: event.target.value
                    }))
                  }
                  placeholder="默认 20"
                />
              </Field>
              <Field label="环境变量" wide>
                <textarea
                  rows={4}
                  value={service.stop_command.env_text}
                  onChange={(event) =>
                    updateServiceStopCommand(serviceIndex, (current) => ({
                      ...current,
                      env_text: event.target.value
                    }))
                  }
                  placeholder={"NODE_ENV: development\nPORT: 8081"}
                />
              </Field>
            </div>
          ) : null}
        </div>
      </div>
    );
  }

  function renderComputerPermissionPanel(service: UiServiceConfig) {
    if (!service.enabled) {
      return null;
    }

    return (
      <div className="service-permission-panel">
        <div className="runtime-config-head">
          <div>
            <strong>桌面控制权限</strong>
            <small>桌面控制启用后，截图需要屏幕录制；点击、输入和拖拽需要辅助功能。</small>
          </div>
          <button className="ghost" onClick={() => void refreshDesktopPermissions()}>
            刷新状态
          </button>
        </div>
        <div className="status-detail-grid">
          <InfoRow
            label="屏幕录制"
            value={formatDesktopPermissionValue(desktopPermissions, "screen_recording", "用于截图")}
            tone={
              desktopPermissions?.screenRecordingSupported &&
              !desktopPermissions.screenRecordingGranted
                ? "danger"
                : "normal"
            }
          />
          <InfoRow
            label="辅助功能"
            value={formatDesktopPermissionValue(desktopPermissions, "accessibility", "用于点击、输入和拖拽")}
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
      </div>
    );
  }

  function renderServiceDefinitionJson(service: UiServiceConfig, serviceIndex: number) {
    const draft = serviceJsonDrafts[serviceIndex] ?? serviceCapabilitiesJson(service);
    const error = serviceJsonErrors[serviceIndex];

    return (
      <div className="service-json-section">
        <div className="method-advanced-head">
          <strong>能力定义 JSON</strong>
          <small>统一编辑 methods 和 events；保存应用时会按同一套配置校验写入。</small>
        </div>
        <textarea
          className="json-editor"
          rows={18}
          value={draft}
          onChange={(event) => updateServiceJsonDraft(serviceIndex, event.target.value)}
        />
        {error ? <div className="json-error">{error}</div> : null}
        <div className="json-editor-actions">
          <button className="secondary" onClick={() => applyServiceJsonDraft(serviceIndex)}>
            应用 JSON
          </button>
          <button className="ghost" onClick={() => resetServiceJsonDraft(serviceIndex)}>
            重置
          </button>
        </div>
      </div>
    );
  }

  function renderServiceEditor(service: UiServiceConfig, serviceIndex: number) {
    const isComputer = isComputerService(service);
    const isSystem = isSystemService(service);
    const serviceDirty = savedServiceSignatures[serviceIndex] !== serviceSignature(service);
    const servicePersisted = savedServiceSignatures[serviceIndex] != null;
    const hasRuntimeControls =
      service.health_check != null || service.start_command != null || service.stop_command != null;
    const serviceNotice = serviceNotices[serviceIndex];

    return (
      <Card
        title={service.name || "未命名应用"}
        description={
          isSystem
            ? "系统内置"
            : service.description || "自定义本地应用"
        }
        action={
          <div className="service-actions">
            <span className={`service-save-state ${serviceDirty ? "dirty" : "clean"}`}>
              {serviceDirty ? "未保存" : "已保存"}
            </span>
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
            <button className="secondary" onClick={() => void saveService(serviceIndex)} disabled={busy}>
              保存配置
            </button>
            <button className="primary" onClick={() => void saveService(serviceIndex, true)} disabled={busy}>
              保存并应用
            </button>
            {!isSystem ? (
              <button
                className="ghost danger"
                onClick={() =>
                  servicePersisted
                    ? void deleteSavedService(serviceIndex, true)
                    : removeService(serviceIndex)
                }
                disabled={busy}
              >
                {servicePersisted ? "删除并应用" : "移除草稿"}
              </button>
            ) : null}
          </div>
        }
      >
        {serviceNotice ? <div className="service-local-notice">{serviceNotice}</div> : null}
        {isComputer ? renderComputerPermissionPanel(service) : null}
        {hasRuntimeControls ? renderServiceRuntimePanel(service, serviceIndex, isSystem) : null}
        <div className="service-editor-layout">
          {!isSystem ? (
            <>
              <div className="form-grid">
                <Field label="内部服务名">
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
                <Field label="应用说明">
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
              {renderServiceRuntimeConfig(service, serviceIndex, isSystem)}
              {renderServiceDefinitionJson(service, serviceIndex)}
            </>
          ) : null}
          <div className="method-list">
            {service.methods.map((method, methodIndex) => (
              <div className="method-card" key={`${service.name}-${method.name}-${methodIndex}`}>
                <div className="method-topline">
                  <div className="method-copy">
                    <div className="method-title-row">
                      <h4>{method.name || "未命名方法"}</h4>
                      <span className="method-badge">{formatMethodTypeLabel(method.binding.type)}</span>
                      {method.enabled != null ? (
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
                  {isSystem && !isComputer ? (
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
                      {!isSystem ? (
                        <button
                          className="ghost danger"
                          onClick={() => removeMethod(serviceIndex, methodIndex)}
                        >
                          删除方法
                        </button>
                      ) : null}
                    </div>
                  ) : null}
                </div>

                {isSystem && !isComputer ? (
                  <div className="form-grid">
                    {!isSystem ? (
                      <>
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
                      </>
                    ) : null}

                    {method.binding.type === "computer_use" ? (
                      <Field
                        label="桌面控制能力"
                        hint="内置能力由系统应用维护，不在自定义应用里编辑。"
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

                {isSystem && !isComputer && isMethodAdvancedOpen(serviceIndex, methodIndex) ? (
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

                    {!isSystem ? (
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
                    ) : null}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
          <div className="method-list">
            {service.events.map((eventConfig, eventIndex) => (
              <div className="method-card" key={`${service.name}-${eventConfig.name}-event-${eventIndex}`}>
                <div className="method-topline">
                  <div className="method-copy">
                    <div className="method-title-row">
                      <h4>{eventConfig.name || "未命名事件"}</h4>
                      <span className="method-badge">Event</span>
                      <span className={`service-badge ${eventConfig.enabled ? "enabled" : "disabled"}`}>
                        {eventConfig.enabled ? "启用" : "停用"}
                      </span>
                    </div>
                    <p>{eventConfig.description || "本地自定义应用可通过本机事件入口发送。"}</p>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </Card>
    );
  }

  function renderCapabilityMethodConfig(method: UiMethodConfig, serviceIndex: number, methodIndex: number) {
    if (!isMethodAdvancedOpen(serviceIndex, methodIndex)) {
      return null;
    }
    const configuresShellExecution =
      method.binding.type === "shell_command" &&
      ["exec", "startExecution"].includes(method.name);

    return (
      <div className="method-advanced capability-config-panel">
        <div className="method-advanced-head">
          <strong>方法配置</strong>
          <small>这里直接配置当前能力的方法权限和本地绑定。</small>
        </div>
        <div className="form-grid">
          <Field label="方法状态">
            <label className="switch inline-switch">
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
          </Field>

          {method.binding.type === "computer_use" ? (
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
          ) : null}

          {method.binding.type === "shell_command" ? (
            <>
              {configuresShellExecution ? (
                <>
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
                  <Field label="根目录" hint="默认相对配置目录，填 / 表示整机根目录。">
                    <input
                      value={method.binding.root_dir}
                      onChange={(event) =>
                        updateMethod(serviceIndex, methodIndex, (current) => ({
                          ...current,
                          binding:
                            current.binding.type === "shell_command"
                              ? {
                                  ...current.binding,
                                  root_dir: event.target.value
                                }
                              : current.binding
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
                          binding:
                            current.binding.type === "shell_command"
                              ? {
                                  ...current.binding,
                                  allow_commands_text: event.target.value
                                }
                              : current.binding
                        }))
                      }
                      placeholder="echo, pwd, git 或 *"
                    />
                  </Field>
                </>
              ) : null}
              <Field label="默认超时">
                <input
                  value={method.binding.default_timeout_secs}
                  onChange={(event) =>
                    updateMethod(serviceIndex, methodIndex, (current) => ({
                      ...current,
                      binding:
                        current.binding.type === "shell_command"
                          ? {
                              ...current.binding,
                              default_timeout_secs: event.target.value
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "shell_command"
                          ? {
                              ...current.binding,
                              max_timeout_secs: event.target.value
                            }
                          : current.binding
                    }))
                  }
                  placeholder="留空则使用全局"
                />
              </Field>
            </>
          ) : null}

          {method.binding.type === "http" ? (
            <>
              <Field label="本地 URL" wide>
                <input
                  value={method.binding.url}
                  onChange={(event) =>
                    updateMethod(serviceIndex, methodIndex, (current) => ({
                      ...current,
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              url: event.target.value
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              http_method: event.target.value.toUpperCase()
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              timeout_secs: event.target.value
                            }
                          : current.binding
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
                      binding:
                        current.binding.type === "http"
                          ? {
                              ...current.binding,
                              headers_text: event.target.value
                            }
                          : current.binding
                    }))
                  }
                  placeholder={"Authorization: Bearer xxx\nX-App: local-java"}
                />
              </Field>
            </>
          ) : null}
        </div>
      </div>
    );
  }

  function renderLocalAppAbilityList(app: LocalAppItem, canShowConfig: boolean) {
    if (!config) {
      return <div />;
    }
    const services = app.serviceIndexes
      .map((serviceIndex) => ({ serviceIndex, service: config.services[serviceIndex] }))
      .filter((entry): entry is { serviceIndex: number; service: UiServiceConfig } =>
        Boolean(entry.service)
      );

    if (services.length === 0) {
      return <div className="empty-state compact-empty">应用已安装，但当前没有写入能力。</div>;
    }

    return (
      <div className="app-ability-list">
        {services.map(({ serviceIndex, service }) => (
          <div className="app-ability-group" key={service.name}>
            <div className="app-ability-group-head">
              <div>
                <strong>{service.description || service.name}</strong>
                <span className={`service-badge ${service.enabled ? "enabled" : "disabled"}`}>
                  {service.enabled ? "启用" : "停用"}
                </span>
              </div>
              {canShowConfig ? (
                <div className="service-actions">
                  <span
                    className={`service-save-state ${
                      savedServiceSignatures[serviceIndex] !== serviceSignature(service) ? "dirty" : "clean"
                    }`}
                  >
                    {savedServiceSignatures[serviceIndex] !== serviceSignature(service) ? "未保存" : "已保存"}
                  </span>
                  <button
                    className="secondary compact-config-button"
                    onClick={() => void saveService(serviceIndex)}
                    disabled={busy}
                  >
                    保存配置
                  </button>
                  <button
                    className="primary compact-config-button"
                    onClick={() => void saveService(serviceIndex, true)}
                    disabled={busy}
                  >
                    保存并应用
                  </button>
                </div>
              ) : null}
            </div>
            <div className="method-list compact-method-list">
              {service.methods.map((method, methodIndex) => {
                const testKey = buildCapabilityTestKey(serviceIndex, methodIndex);
                const testDraft = capabilityTestDrafts[testKey] ?? defaultCapabilityArgumentsText(method);
                const testResult = capabilityTestResults[testKey];
                const testDisabled = !service.enabled || !method.enabled || capabilityTestBusy != null;
                const configOpen = isMethodAdvancedOpen(serviceIndex, methodIndex);
                return (
                  <div className="method-card compact-method-card" key={`${service.name}-${method.name}-${methodIndex}`}>
                    <div className="method-topline compact-method-topline">
                      <div className="method-copy">
                        <div className="method-title-row">
                          <h4>{method.name || "未命名方法"}</h4>
                          <span className="method-badge">{formatMethodTypeLabel(method.binding.type)}</span>
                          <span className={`service-badge ${method.enabled ? "enabled" : "disabled"}`}>
                            {method.enabled ? "启用" : "停用"}
                          </span>
                        </div>
                        <p>{method.description || "本地方法"}</p>
                      </div>
                      <div className="capability-card-actions">
                        <button
                          className="primary compact-config-button"
                          onClick={() => void testCapability(serviceIndex, methodIndex)}
                          disabled={testDisabled}
                        >
                          {capabilityTestBusy === testKey ? "测试中" : "测试"}
                        </button>
                        {canShowConfig ? (
                          <button
                            className={
                              configOpen
                                ? "secondary compact-config-button active-toggle"
                                : "secondary compact-config-button"
                            }
                            onClick={() => openLocalAppCapabilityConfig(serviceIndex, methodIndex)}
                          >
                            {configOpen ? "收起配置" : "配置"}
                          </button>
                        ) : null}
                      </div>
                    </div>
                    {canShowConfig ? renderCapabilityMethodConfig(method, serviceIndex, methodIndex) : null}
                    <div className="capability-test-panel">
                      <label>
                        <span>参数 JSON</span>
                        <small>请使用标准 JSON，属性名和字符串必须用英文双引号 "，不能用中文引号 “ ”。</small>
                        <textarea
                          rows={Math.max(3, Math.min(8, testDraft.split("\n").length))}
                          value={testDraft}
                          onChange={(event) =>
                            setCapabilityTestDrafts((current) => ({
                              ...current,
                              [testKey]: event.target.value
                            }))
                          }
                        />
                      </label>
                      {testResult ? (
                        <div className={`capability-test-result ${testResult.status}`}>
                          <div className="capability-test-result-head">
                            <strong>{testResult.message}</strong>
                            {testResult.result ? <span>{testResult.result.duration_ms}ms</span> : null}
                          </div>
                          {testResult.result ? (
                            <pre>{prettyJson(testResult.result.success ? testResult.result.data ?? {} : testResult.result.error ?? {})}</pre>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  </div>
                );
              })}
              {service.events.map((eventConfig, eventIndex) => (
                <div
                  className="method-card compact-method-card"
                  key={`${service.name}-${eventConfig.name}-event-${eventIndex}`}
                >
                  <div className="method-topline compact-method-topline">
                    <div className="method-copy">
                      <div className="method-title-row">
                        <h4>{eventConfig.name || "未命名事件"}</h4>
                        <span className="method-badge">Event</span>
                        <span className={`service-badge ${eventConfig.enabled ? "enabled" : "disabled"}`}>
                          {eventConfig.enabled ? "启用" : "停用"}
                        </span>
                      </div>
                      <p>{eventConfig.description || "本地事件"}</p>
                    </div>
                  </div>
                </div>
              ))}
              {service.methods.length === 0 && service.events.length === 0 ? (
                <div className="empty-state compact-empty">当前没有开放能力。</div>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    );
  }

  function renderLocalAppRuntime(app: LocalAppItem) {
    if (!config) {
      return null;
    }
    const services = app.serviceIndexes
      .map((serviceIndex) => config.services[serviceIndex])
      .filter((service): service is UiServiceConfig => Boolean(service));
    if (services.length === 0) {
      return null;
    }
    const lifecycle = localAppLifecycle(app);

    return (
      <div className="local-app-runtime-list">
        <div className="local-app-lifecycle-row">
          <div>
            <strong>应用生命周期</strong>
            <p>{lifecycle.detail}</p>
          </div>
          <div className={`status-pill status-${lifecycle.statusClass}`}>{lifecycle.label}</div>
        </div>
        {services.map((service) => {
          const runtimeView = serviceRuntimeView(service);
          return (
            <div className="local-app-runtime-row" key={service.name}>
              <div>
                <strong>{service.description || service.name}</strong>
                <p>{runtimeView.detail}</p>
              </div>
              {runtimeView.statusLabel && runtimeView.statusClass ? (
                <div className={`status-pill status-${runtimeView.statusClass}`}>
                  {runtimeView.statusLabel}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    );
  }

  function renderInstallLocalAppPanel() {
    if (!installPanelOpen) {
      return null;
    }
    const selectedMarket = installableMarketConnectors.find((app) => app.id === selectedMarketAppId);
    const closeInstallPanel = () => {
      if (installBusy) {
        return;
      }
      setInstallPanelOpen(false);
      setInstallSourceMode("choose");
    };

    return (
      <div className="modal-backdrop" role="presentation" onClick={closeInstallPanel}>
        <section className="install-panel" role="dialog" aria-modal="true" onClick={(event) => event.stopPropagation()}>
          <div className="install-panel-head">
            <div>
              <p className="eyebrow">安装</p>
              <h3>
                {installSourceMode === "market"
                  ? "从市场安装"
                  : installSourceMode === "custom"
                    ? "自定义安装"
                    : "安装本地应用"}
              </h3>
            </div>
            <button className="ghost" onClick={closeInstallPanel} disabled={installBusy}>
              关闭
            </button>
          </div>

          {installSourceMode === "choose" ? (
            <div className="install-choice-grid">
              <button
                className="install-choice-card"
                onClick={() => {
                  setInstallSourceMode("market");
                  void refreshMarketConnectorApps();
                }}
              >
                <strong>从市场安装</strong>
                <span>选择官方维护的本地应用，安装后直接生效。</span>
              </button>
              <button className="install-choice-card" onClick={() => setInstallSourceMode("custom")}>
                <strong>自定义安装</strong>
                <span>从本地目录或 Git 仓库安装开发中的 connector。</span>
              </button>
            </div>
          ) : installSourceMode === "market" ? (
            <div className="market-app-grid">
              {installableMarketConnectors.map((app) => (
                <button
                  className={`market-app-card ${selectedMarketAppId === app.id ? "active" : ""}`}
                  key={app.id}
                  onClick={() => setSelectedMarketAppId(app.id)}
                >
                  <strong>{app.name}</strong>
                  <span>{app.description}</span>
                  <small>{app.capability} · {app.version}</small>
                </button>
              ))}
              {installableMarketConnectors.length === 0 ? (
                <div className="empty-state">暂时没有可安装的市场应用。</div>
              ) : null}
              {selectedMarket ? (
                <div className="install-risk-note">
                  <strong>权限提示</strong>
                  <span>{selectedMarket.risk}</span>
                </div>
              ) : null}
            </div>
          ) : (
            <div className="form-grid">
              <Field label="本地目录或 Git 仓库" wide>
                <input
                  value={installSource}
                  onChange={(event) => setInstallSource(event.target.value)}
                  placeholder="/Users/me/connectors/my-app 或 https://gitee.com/org/repo.git"
                />
              </Field>
            </div>
          )}

          <div className="install-panel-actions">
            {installSourceMode === "choose" ? (
              <button className="secondary" onClick={closeInstallPanel} disabled={installBusy}>
                取消
              </button>
            ) : (
              <>
                <button className="secondary" onClick={() => setInstallSourceMode("choose")} disabled={installBusy}>
                  返回
                </button>
                <button className="primary" onClick={() => void installLocalApp()} disabled={installBusy}>
                  {installBusy ? "安装中" : "安装应用"}
                </button>
              </>
            )}
          </div>
        </section>
      </div>
    );
  }

  function renderLocalAppPanel() {
    if (!config) {
      return <div />;
    }
    const groupedApps: Array<{ title: string; apps: LocalAppItem[] }> = [
      { title: "官方工具", apps: localApps.filter((app) => app.kind === "managed_tool") },
      { title: "已安装应用", apps: localApps.filter((app) => app.kind === "connector") },
      { title: "内置应用", apps: localApps.filter((app) => app.kind === "built_in") },
      { title: "自定义应用", apps: localApps.filter((app) => app.kind === "custom") }
    ].filter((group) => group.apps.length > 0);

    return (
      <div className="local-app-panel">
        <div className="local-app-panel-head">
          <div>
            <p className="eyebrow">本地应用</p>
            <h2>应用</h2>
            <p>管理官方工具和 connector，查看本机能力、版本和授权状态。</p>
          </div>
          <button
            className="primary"
            onClick={() => {
              setInstallSourceMode("choose");
              setInstallPanelOpen(true);
            }}
          >
            安装应用
          </button>
        </div>
        {groupedApps.length > 0 ? (
          <div className="local-app-groups">
            {groupedApps.map((group) => (
              <section className="local-app-group" key={group.title}>
                <div className="method-advanced-head">
                  <strong>{group.title}</strong>
                  <small>{group.apps.length} 个应用</small>
                </div>
                <div className="local-app-grid">
                  {group.apps.map((app) => renderLocalAppCard(app))}
                </div>
              </section>
            ))}
          </div>
        ) : (
          <Card title="还没有应用" description="从应用市场安装 connector，或从本地目录安装开发中的应用。">
            <div className="empty-state">还没有安装应用。</div>
          </Card>
        )}
      </div>
    );
  }

  function renderLocalAppCard(app: LocalAppItem) {
    if (!config) {
      return null;
    }
    const hasStartCommand = hasLocalAppStartCommand(app);
    const lifecycle = localAppLifecycle(app);
    return (
      <button
        className="local-app-card"
        key={app.id}
        onClick={() => {
          setSelectedLocalAppId(app.id);
          setActiveLocalAppDetailTab("overview");
          if (app.serviceIndexes.length > 0) {
            setExpandedServiceIndex(app.serviceIndexes[0]);
          }
        }}
      >
        <div className="local-app-card-top">
          <strong>{app.name}</strong>
          <span className={`sidebar-service-status status-${lifecycle.statusClass}`}>
            {lifecycle.label}
          </span>
        </div>
        <p>{app.description}</p>
        <div className="local-app-card-meta">
          <span>{formatLocalAppKind(app.kind)}</span>
          {app.managedTool ? (
            <span>版本 {app.managedTool.installedVersion ?? "未安装"}</span>
          ) : (
            <span>{countLocalAppCapabilities(app, config)} 项能力</span>
          )}
          {app.codexAccountManagement ? (
            <span>{codexCredentialManager?.activeProfile?.workspaceName ?? "未配置工作区"}</span>
          ) : null}
          {hasStartCommand ? <span>可启动</span> : null}
        </div>
      </button>
    );
  }

  function renderCodexCredentialManager() {
    const state = codexCredentialManager;
    const active = state?.activeProfile;
    const credentialStatus = formatCodexCredentialStatus(state?.credentialStatus);
    return (
      <div className="codex-credential-manager">
        <div className="status-detail-grid">
          <InfoRow label="当前工作区" value={active ? `${active.workspaceName}（${active.workspaceId}）` : "尚未识别"} />
          <InfoRow
            label="当前项目"
            value={active ? `${active.projectName || "项目"}（${active.projectId}）` : "尚未识别"}
          />
          <InfoRow label="模型" value={active?.model ?? "gpt-5.6-sol"} />
          <InfoRow
            label="凭证状态"
            value={credentialStatus.label}
            tone={credentialStatus.tone}
          />
        </div>

        {codexCredentialError ? <div className="error-banner">{codexCredentialError}</div> : null}
        {state?.discoveryWarning ? <div className="notice-banner warning">{state.discoveryWarning}</div> : null}

        <section className="codex-switch-panel">
          <div className="method-advanced-head">
            <div>
              <strong>切换工作区</strong>
              <small>切换时重新签发凭证，不在本地保存多份明文 LLM key。</small>
            </div>
            <button
              className="secondary"
              onClick={() => void refreshCodexCredentialManager()}
              disabled={codexCredentialBusy}
            >
              刷新状态
            </button>
          </div>
          <div className="form-grid">
            <Field label="工作区" wide>
              <select
                value={codexWorkspaceId}
                onChange={(event) => selectCodexWorkspace(state, Number(event.target.value))}
                disabled={codexCredentialBusy || !state}
              >
                <option value="">选择工作区</option>
                {state?.workspaces.map((workspace) => (
                  <option value={workspace.workspaceId} key={workspace.workspaceId}>
                    {workspace.name}（{workspace.workspaceId}）
                  </option>
                ))}
              </select>
            </Field>
            <Field
              label="项目 ID"
              hint={codexProjectsBusy ? "正在读取项目列表…" : "必填，用于模型调用归属、计量和审计。"}
            >
              <>
                <input
                  type="number"
                  min="1"
                  list="codex-project-options"
                  value={codexProjectId}
                  onChange={(event) => {
                    const value = event.target.value;
                    setCodexProjectId(value);
                    const project = codexProjects.find((item) => item.projectId === Number(value));
                    if (project) {
                      setCodexProjectName(project.name);
                    }
                  }}
                  placeholder="例如 7405"
                  disabled={codexCredentialBusy}
                />
                <datalist id="codex-project-options">
                  {codexProjects.map((project) => (
                    <option value={project.projectId} key={project.projectId}>{project.name}</option>
                  ))}
                </datalist>
              </>
            </Field>
            <Field label="项目名称" hint="可选，仅用于本机显示。">
              <input
                value={codexProjectName}
                onChange={(event) => setCodexProjectName(event.target.value)}
                placeholder="项目名称"
                disabled={codexCredentialBusy}
              />
            </Field>
          </div>
          {codexProjectsError ? <p className="field-error">{codexProjectsError}</p> : null}
          <div className="codex-switch-actions">
            <span>应用会先校验凭证归属，再更新配置并重启 Codex。</span>
            <button
              className="primary"
              onClick={() => void switchCodexCredential()}
              disabled={codexCredentialBusy || !state}
            >
              {codexCredentialBusy ? "正在切换" : "签发并切换"}
            </button>
          </div>
        </section>

        <section className="codex-profile-panel">
          <div className="method-advanced-head">
            <strong>最近使用</strong>
            <small>{state?.profiles.length ?? 0} 个工作区 / 项目配置</small>
          </div>
          {state?.profiles.length ? (
            <div className="codex-profile-list">
              {state.profiles.map((profile) => {
                const isActive = active?.workspaceId === profile.workspaceId && active?.projectId === profile.projectId;
                return (
                  <div className={`codex-profile-row ${isActive ? "active" : ""}`} key={`${profile.workspaceId}:${profile.projectId}`}>
                    <div>
                      <strong>{profile.workspaceName}</strong>
                      <span>{profile.projectName || `项目 ${profile.projectId}`} · {profile.model}</span>
                    </div>
                    <button
                      className={isActive ? "secondary" : "primary"}
                      onClick={() => void switchCodexCredential(profile)}
                      disabled={codexCredentialBusy || isActive}
                    >
                      {isActive ? "当前使用" : "切换"}
                    </button>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="empty-state">还没有由本地应用管理的工作区配置。</div>
          )}
        </section>

        {showAdvancedSettings && state ? (
          <div className="status-detail-grid codex-path-grid">
            <InfoRow label="Codex auth" value={state.codexAuthPath} />
            <InfoRow label="Codex config" value={state.codexConfigPath} />
            <InfoRow label="百积木授权" value={state.sharedAuthPath} />
          </div>
        ) : null}
      </div>
    );
  }

  function renderLocalAppDetailDialog(app: LocalAppItem) {
    if (!config) {
      return null;
    }
    const appComputerService = app
      .serviceIndexes.map((serviceIndex) => config.services[serviceIndex])
      .find((service): service is UiServiceConfig => Boolean(service && isComputerService(service)));
    const hasShellCapability = app.serviceIndexes.some((serviceIndex) => {
      const service = config.services[serviceIndex];
      return service ? isShellService(service) : false;
    });
    const hasCodexAccountManagement = app.codexAccountManagement === true;
    const isManagedTool = app.kind === "managed_tool" && Boolean(app.managedTool);
    const canConfigureCapabilities =
      !isManagedTool && (showAdvancedSettings || app.kind === "custom" || hasShellCapability);
    const canShowDeveloperConfig = !isManagedTool && (showAdvancedSettings || app.kind === "custom");
    const marketApp = marketConnectorForLocalApp(app);
    const marketTool = marketManagedToolForLocalApp(app);
    const updateStatus = app.connector ? connectorUpdateStatuses[app.connector.id] : undefined;
    const updateBusy = connectorUpdateBusy === app.id;
    const lifecycle = localAppLifecycle(app);
    const appIsRunning = lifecycle.state === "running";
    const appCanStop = hasLocalAppStopCommand(app);
    const syncSource = connectorSyncSource(app);
    const closeDetail = () => setSelectedLocalAppId(null);

    return (
      <div className="modal-backdrop" role="presentation" onClick={closeDetail}>
        <section
          className="app-detail-dialog"
          role="dialog"
          aria-modal="true"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="install-panel-head">
            <div>
              <p className="eyebrow">{formatLocalAppKind(app.kind)}</p>
              <h3>{app.name}</h3>
              <p>{app.description}</p>
            </div>
            <button className="ghost" onClick={closeDetail}>
              关闭
            </button>
          </div>

          <div className="app-detail-toolbar">
            <div className="section-tabs">
              <button
                className={`section-tab ${activeLocalAppDetailTab === "overview" ? "active" : ""}`}
                onClick={() => setActiveLocalAppDetailTab("overview")}
              >
                概览
              </button>
              {hasCodexAccountManagement ? (
                <button
                  className={`section-tab ${activeLocalAppDetailTab === "account" ? "active" : ""}`}
                  onClick={() => setActiveLocalAppDetailTab("account")}
                >
                  账户与工作区
                </button>
              ) : null}
              {!isManagedTool ? (
                <button
                  className={`section-tab ${activeLocalAppDetailTab === "capabilities" ? "active" : ""}`}
                  onClick={() => setActiveLocalAppDetailTab("capabilities")}
                >
                  能力
                </button>
              ) : null}
              {canShowDeveloperConfig && !isManagedTool ? (
                <button
                  className={`section-tab ${activeLocalAppDetailTab === "config" ? "active" : ""}`}
                  onClick={() => setActiveLocalAppDetailTab("config")}
                >
                  配置
                </button>
              ) : null}
            </div>
            <div className="service-actions">
              {isManagedTool && app.managedTool ? (
                <>
                  <button
                    className="primary accent"
                    onClick={() => void upgradeManagedTool(app)}
                    disabled={managedToolBusy || !marketTool}
                  >
                    {managedToolBusy
                      ? "处理中"
                      : app.managedTool.state === "ready"
                        ? marketTool && compareVersions(marketTool.version, app.managedTool.installedVersion) > 0
                          ? `升级到 ${marketTool.version}`
                          : "校验并修复"
                        : "安装工具"}
                  </button>
                  {app.managedTool.canRollback ? (
                    <button
                      className="secondary"
                      onClick={() => void rollbackManagedTool(app)}
                      disabled={managedToolBusy}
                    >
                      回滚到 {app.managedTool.previousVersion}
                    </button>
                  ) : null}
                </>
              ) : app.connector && marketApp ? (
                updateStatus?.updateAvailable ? (
                  <button
                    className="primary accent"
                    onClick={() => void upgradeLocalApp(app)}
                    disabled={connectorBusy != null || connectorUpdateBusy != null}
                  >
                    {updateBusy ? "升级中" : `升级到 ${updateStatus.latestVersion}`}
                  </button>
                ) : (
                  <button
                    className="secondary"
                    onClick={() => void checkLocalAppUpdate(app)}
                    disabled={connectorBusy != null || connectorUpdateBusy != null}
                  >
                    {updateBusy ? "检查中" : "检查更新"}
                  </button>
                )
              ) : app.connector ? (
                <button
                  className="secondary"
                  onClick={() => void syncLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null || !syncSource}
                >
                  {updateBusy ? "同步中" : "拉取最新"}
                </button>
              ) : null}
              {appIsRunning && appCanStop ? (
                <button
                  className="primary danger"
                  onClick={() => void stopLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  {connectorBusy === app.id ? "停止中" : "停止应用"}
                </button>
              ) : hasLocalAppStartCommand(app) ? (
                <button
                  className="primary"
                  onClick={() => void startLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  {connectorBusy === app.id ? "启动中" : "启动应用"}
                </button>
              ) : null}
              {app.kind === "connector" ? (
                <button
                  className="ghost danger"
                  onClick={() => void uninstallLocalApp(app)}
                  disabled={connectorBusy != null || connectorUpdateBusy != null}
                >
                  卸载
                </button>
              ) : null}
            </div>
          </div>

          {activeLocalAppDetailTab === "overview" ? (
            <div className="app-detail-tab-panel">
              <div className="status-detail-grid">
                <InfoRow label="类型" value={formatLocalAppKind(app.kind)} />
                <InfoRow
                  label="来源类型"
                  value={isManagedTool ? "官方独立发行" : app.connector ? connectorSourceKind(app, marketApp) : "内置"}
                />
                <InfoRow label="安装来源" value={isManagedTool ? marketTool?.source ?? "等待市场版本" : syncSource || "内置"} />
                <InfoRow label="安装位置" value={app.managedTool?.activePath ?? app.connector?.packagePath ?? "随客户端发布"} />
                <InfoRow label="版本" value={app.managedTool?.installedVersion ?? app.connector?.version ?? "随客户端发布"} />
                {app.managedTool ? (
                  <>
                    <InfoRow label="稳定命令" value={app.managedTool.launcherPath} />
                    <InfoRow label="随包基线" value={app.managedTool.bundledVersion ?? "无"} />
                    <InfoRow label="上一版本" value={app.managedTool.previousVersion ?? "无"} />
                    <InfoRow label="状态" value={app.managedTool.detail} />
                  </>
                ) : null}
                {app.connector ? (
                  <InfoRow label="上次同步" value={formatTime(app.connector.lastSyncedAtEpochMs)} />
                ) : null}
                {updateStatus ? (
                  <InfoRow
                    label="更新"
                    value={
                      updateStatus.updateAvailable
                        ? `可升级到 ${updateStatus.latestVersion}`
                        : `已是最新版本 ${updateStatus.currentVersion}`
                    }
                  />
                ) : null}
                {!isManagedTool ? (
                  <InfoRow label="能力数" value={String(countLocalAppCapabilities(app, config))} />
                ) : null}
              </div>
              {appComputerService ? renderComputerPermissionPanel(appComputerService) : null}
              {renderLocalAppRuntime(app)}
              {hasCodexAccountManagement ? (
                <section className="codex-overview-panel">
                  <div className="method-advanced-head">
                    <div>
                      <strong>账户与工作区</strong>
                      <small>本机凭证由 Bridge Agent 管理，不会作为 Connector 能力对外开放。</small>
                    </div>
                    <button className="secondary" onClick={() => setActiveLocalAppDetailTab("account")}>
                      管理
                    </button>
                  </div>
                  <div className="status-detail-grid">
                    <InfoRow
                      label="当前工作区"
                      value={codexCredentialManager?.activeProfile?.workspaceName ?? "尚未配置"}
                    />
                    <InfoRow
                      label="当前项目"
                      value={codexCredentialManager?.activeProfile?.projectName ||
                        (codexCredentialManager?.activeProfile
                          ? `项目 ${codexCredentialManager.activeProfile.projectId}`
                          : "尚未配置")}
                    />
                    <InfoRow
                      label="凭证状态"
                      value={formatCodexCredentialStatus(codexCredentialManager?.credentialStatus).label}
                      tone={formatCodexCredentialStatus(codexCredentialManager?.credentialStatus).tone}
                    />
                  </div>
                  {codexCredentialError ? <div className="error-banner">{codexCredentialError}</div> : null}
                </section>
              ) : null}
            </div>
          ) : null}

          {activeLocalAppDetailTab === "account" && hasCodexAccountManagement ? (
            <div className="app-detail-tab-panel">{renderCodexCredentialManager()}</div>
          ) : null}

          {activeLocalAppDetailTab === "capabilities" && !isManagedTool ? (
            <div className="app-detail-tab-panel">
              <div className="method-advanced-head">
                <strong>能力</strong>
                <small>这些能力会在授权后开放给工作区调用。</small>
              </div>
              {renderLocalAppAbilityList(app, canConfigureCapabilities)}
            </div>
          ) : null}

          {activeLocalAppDetailTab === "config" && canShowDeveloperConfig ? (
            <div className="app-detail-tab-panel developer-config-stack">
              <div className="method-advanced-head">
                <strong>开发者配置</strong>
                <small>内部运行项、启动命令、HTTP 绑定和 JSON 定义。</small>
              </div>
              {app.serviceIndexes.map((serviceIndex) =>
                config.services[serviceIndex] ? renderServiceEditor(config.services[serviceIndex], serviceIndex) : null
              )}
            </div>
          ) : null}
        </section>
      </div>
    );
  }

  function renderAppsPage() {
    return (
      <div className="service-editor-panel">
        {renderLocalAppPanel()}
        {selectedLocalApp ? renderLocalAppDetailDialog(selectedLocalApp) : null}
      </div>
    );
  }

  function hasLocalAppStartCommand(app: LocalAppItem) {
    if (!config) {
      return false;
    }
    return app.serviceIndexes.some((serviceIndex) => Boolean(config.services[serviceIndex]?.start_command));
  }

  function hasLocalAppStopCommand(app: LocalAppItem) {
    if (!config) {
      return false;
    }
    return app.serviceIndexes.some((serviceIndex) => Boolean(config.services[serviceIndex]?.stop_command));
  }

  function setLocalAppLifecycleOverride(appId: string, override: LocalAppLifecycleOverride) {
    setLocalAppLifecycleOverrides((current) => ({
      ...current,
      [appId]: override
    }));
  }

  function localAppLifecycle(app: LocalAppItem): LocalAppLifecycle {
    if (!config) {
      return formatLocalAppLifecycle("unknown", "等待配置加载");
    }

    if (app.managedTool) {
      return formatLocalAppLifecycle(app.managedTool.state, app.managedTool.detail);
    }

    const override = localAppLifecycleOverrides[app.id];
    if (override && ["starting", "stopping", "start_failed", "stopped"].includes(override.state)) {
      return formatLocalAppLifecycle(override.state, override.detail);
    }

    const services = app.serviceIndexes
      .map((serviceIndex) => config.services[serviceIndex])
      .filter((service): service is UiServiceConfig => Boolean(service));
    if (services.length === 0) {
      return formatLocalAppLifecycle("installed", "已安装，尚未关联本地服务");
    }

    const statuses = services
      .map((service) => registeredServiceStatuses.find((status) => status.service === service.name))
      .filter((status): status is RegisteredServiceStatus => Boolean(status));
    if (statuses.some((status) => status.status === "healthy")) {
      return formatLocalAppLifecycle("running", "healthCheck 已通过");
    }
    if (override?.state === "running") {
      return formatLocalAppLifecycle("running", override.detail ?? "启动命令已执行");
    }
    if (statuses.some((status) => status.status === "unhealthy")) {
      return formatLocalAppLifecycle("stopped", "healthCheck 未通过");
    }
    if (statuses.some((status) => status.status === "unknown")) {
      return formatLocalAppLifecycle("unknown", "等待运行状态检查");
    }
    if (hasLocalAppStartCommand(app)) {
      return formatLocalAppLifecycle("installed", "已安装，等待手动启动");
    }
    return formatLocalAppLifecycle("installed", "已安装");
  }

  function isLocalAppRunning(app: LocalAppItem) {
    return localAppLifecycle(app).state === "running";
  }

  function countLocalAppCapabilities(app: LocalAppItem, agentConfig: UiAgentConfig) {
    return app.serviceIndexes.reduce((count, serviceIndex) => {
      const service = agentConfig.services[serviceIndex];
      return service ? count + service.methods.length + service.events.length : count;
    }, 0);
  }

  function formatLocalAppKind(kind: LocalAppKind) {
    const labels: Record<LocalAppKind, string> = {
      connector: "已安装应用",
      managed_tool: "官方工具",
      built_in: "内置应用",
      custom: "自定义应用"
    };
    return labels[kind];
  }

  function compareVersions(left: string, right?: string | null): number {
    if (!right) {
      return 1;
    }
    const normalize = (value: string) =>
      value
        .replace(/^v/i, "")
        .split(/[.-]/)
        .map((part) => Number.parseInt(part, 10))
        .map((part) => (Number.isFinite(part) ? part : 0));
    const leftParts = normalize(left);
    const rightParts = normalize(right);
    for (let index = 0; index < Math.max(leftParts.length, rightParts.length); index += 1) {
      const delta = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
      if (delta !== 0) {
        return delta;
      }
    }
    return 0;
  }

  function renderLogMetadata(entry: LogEntry) {
    const items = [
      entry.category,
      entry.service,
      entry.method,
      entry.event,
      entry.outcome,
      entry.request_id ? `request ${entry.request_id}` : null,
      entry.event_id ? `event ${entry.event_id}` : null,
      entry.duration_ms != null ? `${entry.duration_ms}ms` : null,
      entry.http_method && entry.path ? `${entry.http_method} ${entry.path}` : null,
      entry.status_code != null ? `${entry.status_code}` : null
    ].filter((item): item is string => Boolean(item));

    if (items.length === 0) {
      return null;
    }

    return (
      <div className="log-meta">
        {items.map((item) => (
          <span key={item}>{item}</span>
        ))}
      </div>
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
            <div className="service-actions log-actions">
              <select value={logServiceFilter} onChange={(event) => setLogServiceFilter(event.target.value)}>
                <option value="">全部服务</option>
                {logServiceOptions.map((serviceName) => (
                  <option value={serviceName} key={serviceName}>
                    {serviceName}
                  </option>
                ))}
              </select>
              <button className="ghost" onClick={() => void clearLogs()}>
                清空日志
              </button>
            </div>
          }
        >
          <div className="log-panel">
            {filteredLogs.length === 0 ? (
              <div className="empty-state">暂无日志</div>
            ) : (
              filteredLogs.map((entry, index) => (
                <div className={`log-line log-${entry.level}`} key={`${entry.timestamp_ms}-${index}`}>
                  <span>{formatTime(entry.timestamp_ms)}</span>
                  <strong>{entry.level.toUpperCase()}</strong>
                  <div>
                    <p>{entry.message}</p>
                    {renderLogMetadata(entry)}
                  </div>
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

    if (activeDetailPanel === "settings") {
      return (
        <Card
          title="高级设置"
          description="设备身份、授权连接和本机运行参数。"
          action={
            <div className="service-actions">
              <button className="secondary" onClick={() => void saveConfig()} disabled={busy}>
                保存配置
              </button>
              <button className="secondary" onClick={() => void beginBrowserAuth()} disabled={busy}>
                浏览器授权
              </button>
            </div>
          }
        >
          <div className="status-detail-grid connection-summary-grid">
            <InfoRow label="工作区" value={config.platform.workspace_id || "未授权"} />
            <InfoRow label="平台" value={DEFAULT_PLATFORM_BASE_URL} />
            <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
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

    const isCheckingUpdate = appUpdateCheckState === "checking";

    return (
      <Card title="系统" description="版本与运行状态。">
        <div className="app-version-panel">
          <div>
            <span>当前版本</span>
            <strong>{appVersionLabel}</strong>
            <p className={appUpdateTone === "danger" ? "danger-text" : undefined}>{appUpdateStatusLabel}</p>
          </div>
          <div className="app-version-actions">
            {appUpdate?.updateAvailable ? (
              appUpdate.autoDownloadAvailable ? (
                <button className="primary" onClick={() => void installAppUpdate()} disabled={updateBusy}>
                  {updateBusy ? formatAppUpdateProgressButton(appUpdateProgress) : `升级到 ${appUpdate.latestVersion}`}
                </button>
              ) : (
                <button className="primary" onClick={() => void openExternalUrl(appUpdate.releaseUrl)}>
                  打开下载页
                </button>
              )
            ) : null}
            <button
              className="secondary"
              onClick={() => void checkAppUpdate(true)}
              disabled={updateBusy || isCheckingUpdate}
            >
              {isCheckingUpdate ? "检查中" : "检查更新"}
            </button>
          </div>
        </div>
        {renderAppUpdateProgress()}
        <div className="status-detail-grid">
          <InfoRow label="当前状态" value={statusLabel} />
          <InfoRow
            label="Relay 注册"
            value={formatRelayRegistration(runtime)}
            tone={runtime?.relay_registered ? "normal" : "warning"}
          />
          <InfoRow label="Relay 最近响应" value={formatRelaySeen(runtime)} />
          <InfoRow label="最近事件" value={runtime ? formatTime(runtime.last_event_at) : "-"} />
          <InfoRow label="运行名称" value={runtime?.agent_id ?? config.relay.agent_id} />
          <InfoRow label="Relay" value={runtime?.relay_url ?? config.relay.url} />
          <InfoRow label="日志文件" value={runtime?.log_file_path ?? "未启用"} />
          <InfoRow label="配置文件" value={configPath} />
          <InfoRow
            label="最近错误"
            value={needsAuthorization ? "无" : runtime?.last_error || "无"}
            tone={!needsAuthorization && runtime?.last_error ? "danger" : "normal"}
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
            className={`section-tab ${activeDetailPanel === "settings" ? "active" : ""}`}
            onClick={() => setActiveDetailPanel("settings")}
          >
            设置
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

  function renderToastStack() {
    if (!message && !error) {
      return null;
    }

    return (
      <div className="toast-stack" aria-live="polite" aria-atomic="true">
        {message ? (
          <div className="toast toast-success" role="status">
            <div>
              <strong>已完成</strong>
              <p>{message}</p>
            </div>
            <button className="toast-close" onClick={() => setMessage("")} aria-label="关闭成功提示">
              ×
            </button>
          </div>
        ) : null}
        {error ? (
          <div className="toast toast-error" role="alert">
            <div>
              <strong>操作失败</strong>
              <p>{error}</p>
            </div>
            <button className="toast-close" onClick={() => setError("")} aria-label="关闭错误提示">
              ×
            </button>
          </div>
        ) : null}
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
          <p className="eyebrow">百积木</p>
          <h1>正在加载</h1>
          <p>读取配置和运行状态。</p>
          {error ? (
            <>
              <div className="alert error">{error}</div>
              <div className="loading-actions">
                <button className="primary" onClick={() => void recoverInvalidConfig()} disabled={busy}>
                  {busy ? "恢复中" : "恢复默认配置"}
                </button>
                <button className="secondary" onClick={() => void refreshAll()} disabled={busy}>
                  重试加载
                </button>
              </div>
              <p className="loading-hint">
                恢复时会先把当前配置文件重命名保留，再生成新的默认配置。
              </p>
            </>
          ) : null}
        </section>
      </main>
    );
  }

  function renderForceUpdateOverlay() {
    if (!appUpdate?.forceUpdateRequired) {
      return null;
    }
    const targetVersion = appUpdate.latestVersion ?? appUpdate.minimumSupportedVersion ?? "最新版本";
    const message =
      appUpdate.forceUpdateMessage ||
      `当前版本 ${appUpdate.currentVersion} 已停止支持，需要升级到 ${targetVersion} 后继续使用。`;

    return (
      <div className="force-update-overlay" role="dialog" aria-modal="true" aria-labelledby="force-update-title">
        <section className="force-update-panel">
          <p className="eyebrow">必须更新</p>
          <h2 id="force-update-title">请升级百积木本地连接客户端</h2>
          <p>{message}</p>
          <div className="force-update-meta">
            <InfoRow label="当前版本" value={appUpdate.currentVersion} tone="warning" />
            <InfoRow label="目标版本" value={targetVersion} />
          </div>
          <div className="force-update-actions">
            {appUpdate.autoDownloadAvailable ? (
              <button className="primary danger" onClick={() => void installAppUpdate()} disabled={updateBusy}>
                {updateBusy ? formatAppUpdateProgressButton(appUpdateProgress) : "立即更新"}
              </button>
            ) : (
              <button className="primary danger" onClick={() => void openExternalUrl(appUpdate.releaseUrl)}>
                打开下载页
              </button>
            )}
            <button
              className="secondary"
              onClick={() => void checkAppUpdate(true)}
              disabled={updateBusy || appUpdateCheckState === "checking"}
            >
              {appUpdateCheckState === "checking" ? "检查中" : "重新检查"}
            </button>
          </div>
          {renderAppUpdateProgress()}
        </section>
      </div>
    );
  }

  const pageTitleMap: Record<AppPage, string> = {
    overview: "概览",
    apps: "本地应用",
    diagnostics: "诊断"
  };

  const pageDescriptionMap: Record<AppPage, string> = {
    overview: "",
    apps: "安装、启动和授权本机应用",
    diagnostics: "系统、日志与清单"
  };
  const showPageHeader = activePage !== "apps";

  return (
    <main className="app-shell">
      <div className="desktop-shell">
        <aside className="sidebar">
          <div className="sidebar-brand">
            <div className="sidebar-brand-heading">
              <img className="sidebar-brand-logo" src={bjmLogoLight} alt="" aria-hidden="true" />
              <div>
                <p className="eyebrow">百积木</p>
                <h1>本地连接客户端</h1>
              </div>
            </div>
            <p className="sidebar-device-name">{config.device.name}</p>
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
              className={`sidebar-nav-item ${activePage === "apps" ? "active" : ""}`}
              onClick={() => {
                setActivePage("apps");
                setSelectedLocalAppId(null);
              }}
            >
              <span>应用</span>
              <small>市场与本机</small>
            </button>
            <button
              className={`sidebar-nav-item ${activePage === "diagnostics" ? "active" : ""}`}
              onClick={() => setActivePage("diagnostics")}
            >
              <span>诊断</span>
              <small>{latestLog ? "查看日志与清单" : "系统信息"}</small>
            </button>
          </nav>

        </aside>

        <section className="main-panel">
          {showPageHeader ? (
            <header className="page-header">
              <div>
                <p className="eyebrow">{pageTitleMap[activePage]}</p>
                <h2>{pageTitleMap[activePage]}</h2>
                {pageDescriptionMap[activePage] ? <p>{pageDescriptionMap[activePage]}</p> : null}
              </div>
              <div className="page-actions">
                {activePage === "diagnostics" ? (
                  <button className="ghost" onClick={() => void resetExampleConfig()} disabled={busy}>
                    恢复示例
                  </button>
                ) : null}
              </div>
            </header>
          ) : null}

          {renderRuntimeConflictPanel()}
          {runtime?.last_error && !needsAuthorization && activePage !== "diagnostics" ? (
            <div className="alert warning">{runtime.last_error}</div>
          ) : null}
          {renderBrowserAuthPanel()}

          <div className="page-body">
            {activePage === "overview" ? renderOverviewPage() : null}
            {activePage === "apps" ? renderAppsPage() : null}
            {activePage === "diagnostics" ? renderDiagnosticsPage() : null}
          </div>
        </section>
      </div>
      {renderToastStack()}
      {renderInstallLocalAppPanel()}
      {renderForceUpdateOverlay()}
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
  tone?: "normal" | "warning" | "danger";
}) {
  const valueClass =
    props.tone === "danger" ? "danger-text" : props.tone === "warning" ? "warning-text" : "";
  return (
    <div className="info-row">
      <span>{props.label}</span>
      <strong className={valueClass}>{props.value}</strong>
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
    services: config.services.map(toUiService)
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
    services: config.services.map(fromUiService)
  };
}

function toUiService(service: ServiceConfig): UiServiceConfig {
  return {
    name: service.name,
    description: service.description,
    enabled: service.enabled,
    health_check: service.health_check ? toUiServiceHealthCheck(service.health_check) : null,
    start_command: service.start_command ? toUiServiceStartCommand(service.start_command) : null,
    stop_command: service.stop_command ? toUiServiceStartCommand(service.stop_command) : null,
    events: (service.events ?? []).map(toUiEvent),
    methods: service.methods.map(toUiMethod)
  };
}

function fromUiService(service: UiServiceConfig): ServiceConfig {
  return {
    name: service.name.trim(),
    description: service.description.trim(),
    enabled: service.enabled,
    health_check: service.health_check ? fromUiServiceHealthCheck(service.health_check) : null,
    start_command: service.start_command ? fromUiServiceStartCommand(service.start_command) : null,
    stop_command: service.stop_command ? fromUiServiceStartCommand(service.stop_command) : null,
    events: service.events.map(fromUiEvent),
    methods: service.methods.map(fromUiMethod)
  };
}

function serviceCapabilitiesJson(service: UiServiceConfig): string {
  const document: ServiceCapabilitiesDocument = {
    methods: service.methods.map(fromUiMethod),
    events: service.events.map(fromUiEvent)
  };
  return prettyJson(document);
}

function toUiMethod(method: MethodConfig): UiMethodConfig {
  return {
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
  };
}

function fromUiMethod(method: UiMethodConfig): MethodConfig {
  return {
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
  };
}

function toUiEvent(eventConfig: EventConfig): UiEventConfig {
  return {
    name: eventConfig.name,
    description: eventConfig.description,
    enabled: eventConfig.enabled,
    payload_schema_text: prettyJson(eventConfig.payload_schema)
  };
}

function fromUiEvent(eventConfig: UiEventConfig): EventConfig {
  return {
    name: eventConfig.name.trim(),
    description: eventConfig.description.trim(),
    enabled: eventConfig.enabled,
    payload_schema: parseJson(eventConfig.payload_schema_text)
  };
}

function normalizePlatformBaseUrl(value: string): string {
  const normalized = value.trim();
  if (!normalized) {
    return DEFAULT_PLATFORM_BASE_URL;
  }
  try {
    const url = new URL(normalized);
    const host = url.hostname.replace(/^www\./, "");
    const path = url.pathname.replace(/\/+$/, "");
    if (
      host === "baijimu.com" &&
      (path === "" || path === "/lowcode" || path === "/manager" || path === "/lowcode3")
    ) {
      return DEFAULT_PLATFORM_BASE_URL;
    }
  } catch {
    return normalized;
  }
  return normalized.replace(/\/+$/, "");
}

function buildConsoleUrl(config: UiAgentConfig): string {
  try {
    return new URL("/manager", normalizePlatformBaseUrl(config.platform.base_url)).toString();
  } catch {
    return new URL("/manager", DEFAULT_PLATFORM_BASE_URL).toString();
  }
}

function emptyToNull(value: string): string | null {
  const normalized = value.trim();
  return normalized ? normalized : null;
}

function createShellMethod(): UiMethodConfig {
  return {
    name: "exec",
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

function createEvent(): UiEventConfig {
  return {
    name: "jobFinished",
    description: "Emitted when the local service completes an asynchronous job.",
    enabled: true,
    payload_schema_text: prettyJson(EMPTY_OBJECT_SCHEMA)
  };
}

function createServiceHealthCheck(): UiServiceHealthCheck {
  return {
    type: "http",
    url: "http://127.0.0.1:8081/health",
    http_method: "GET",
    headers_text: "",
    timeout_secs: "3",
    expect_status: "200",
    body_contains: ""
  };
}

function createServiceStartCommand(): UiServiceStartCommand {
  return {
    type: "shell_command",
    command_text: "npm\nrun\ndev",
    cwd: "",
    env_text: "",
    timeout_secs: "20"
  };
}

function createServiceStopCommand(): UiServiceStartCommand {
  return {
    type: "shell_command",
    command_text: "",
    cwd: "",
    env_text: "",
    timeout_secs: "20"
  };
}

function toUiServiceHealthCheck(healthCheck: ServiceHealthCheck): UiServiceHealthCheck {
  return {
    type: "http",
    url: healthCheck.url,
    http_method: healthCheck.http_method,
    headers_text: headersToText(healthCheck.headers),
    timeout_secs: toOptionalText(healthCheck.timeout_secs),
    expect_status: toOptionalText(healthCheck.expect_status),
    body_contains: healthCheck.body_contains ?? ""
  };
}

function fromUiServiceHealthCheck(healthCheck: UiServiceHealthCheck): ServiceHealthCheck {
  return {
    type: "http",
    url: healthCheck.url.trim(),
    http_method: healthCheck.http_method.trim().toUpperCase(),
    headers: textToHeaders(healthCheck.headers_text),
    timeout_secs: toOptionalNumber(healthCheck.timeout_secs),
    expect_status: toOptionalNumber(healthCheck.expect_status),
    body_contains: emptyToNull(healthCheck.body_contains)
  };
}

function toUiServiceStartCommand(startCommand: ServiceStartCommand): UiServiceStartCommand {
  return {
    type: "shell_command",
    command_text: startCommand.command.join("\n"),
    cwd: startCommand.cwd ?? "",
    env_text: headersToText(startCommand.env),
    timeout_secs: toOptionalText(startCommand.timeout_secs)
  };
}

function fromUiServiceStartCommand(startCommand: UiServiceStartCommand): ServiceStartCommand {
  const command = splitLineList(startCommand.command_text);
  if (command.length === 0) {
    throw new Error("启动命令不能为空");
  }
  return {
    type: "shell_command",
    command,
    cwd: emptyToNull(startCommand.cwd),
    env: textToHeaders(startCommand.env_text),
    timeout_secs: toOptionalNumber(startCommand.timeout_secs)
  };
}

function isComputerService(service: Pick<UiServiceConfig, "name">): boolean {
  return service.name.trim().toLowerCase() === "computer";
}

function isShellService(service: Pick<UiServiceConfig, "name">): boolean {
  return service.name.trim().toLowerCase() === "shell";
}

function isSystemService(service: Pick<UiServiceConfig, "name">): boolean {
  return isComputerService(service) || isShellService(service);
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

function splitCommaList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function splitLineList(value: string): string[] {
  return value
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean);
}

function reindexRecordAfterDelete(
  record: Record<number, string>,
  deletedIndex: number
): Record<number, string> {
  const next: Record<number, string> = {};
  for (const [rawIndex, value] of Object.entries(record)) {
    const index = Number(rawIndex);
    if (!Number.isInteger(index) || index === deletedIndex) {
      continue;
    }
    next[index > deletedIndex ? index - 1 : index] = value;
  }
  return next;
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

function defaultCapabilityArgumentsText(method: UiMethodConfig): string {
  try {
    const schema = parseJson(method.input_schema_text);
    return prettyJson(sampleJsonValue(schema, method.name));
  } catch {
    if (method.binding.type === "shell_command") {
      return prettyJson({
        command: ["echo", "bridge-agent local test"]
      });
    }
    return "{}";
  }
}

function sampleJsonValue(schema: unknown, propertyName = ""): unknown {
  if (!schema || typeof schema !== "object") {
    return {};
  }
  const candidate = schema as {
    type?: unknown;
    enum?: unknown;
    required?: unknown;
    properties?: unknown;
    items?: unknown;
    minimum?: unknown;
    minItems?: unknown;
  };
  if (Array.isArray(candidate.enum) && candidate.enum.length > 0) {
    return candidate.enum[0];
  }

  const type = Array.isArray(candidate.type) ? candidate.type[0] : candidate.type;
  if (type === "object" || candidate.properties) {
    const properties =
      candidate.properties && typeof candidate.properties === "object"
        ? (candidate.properties as Record<string, unknown>)
        : {};
    const required = Array.isArray(candidate.required)
      ? candidate.required.filter((item): item is string => typeof item === "string")
      : Object.keys(properties);
    const result: Record<string, unknown> = {};
    for (const key of required) {
      if (properties[key]) {
        result[key] = sampleJsonValue(properties[key], key);
      }
    }
    return result;
  }

  if (type === "array") {
    if (propertyName === "command") {
      return ["echo", "bridge-agent local test"];
    }
    if (propertyName === "keys") {
      return ["Enter"];
    }
    if (propertyName === "path") {
      return [
        { x: 100, y: 100 },
        { x: 160, y: 160 }
      ];
    }
    return [sampleJsonValue(candidate.items, propertyName)];
  }

  if (type === "integer") {
    if (propertyName === "ms") {
      return 500;
    }
    return typeof candidate.minimum === "number" ? candidate.minimum : 1;
  }
  if (type === "number") {
    if (propertyName === "x" || propertyName === "y") {
      return 100;
    }
    return typeof candidate.minimum === "number" ? candidate.minimum : 1;
  }
  if (type === "boolean") {
    return true;
  }
  if (type === "string") {
    if (propertyName === "text") {
      return "本地测试";
    }
    return "";
  }
  return {};
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

function formatRegisteredServiceDetail(
  service: UiServiceConfig,
  status: RegisteredServiceStatus | undefined
): string {
  const hasStartCommand = status?.startCommandConfigured ?? service.start_command != null;
  const hasStopCommand = status?.stopCommandConfigured ?? service.stop_command != null;
  const hasHealthCheck = status?.healthCheckConfigured ?? service.health_check != null;
  if (!hasHealthCheck && !hasStartCommand && !hasStopCommand) {
    return "仅作为能力清单展示";
  }
  if (!hasHealthCheck && hasStartCommand) {
    return "已配置启动命令；未配置 healthCheck，无法自动确认运行状态";
  }
  return status?.detail ?? "等待状态检查";
}

function formatRegisteredServiceStatus(status: RegisteredServiceState): string {
  const labels: Record<RegisteredServiceState, string> = {
    not_configured: "未配置",
    healthy: "可用",
    unhealthy: "不可用",
    unknown: "未知"
  };
  return labels[status];
}

function formatCodexCredentialStatus(
  status: CodexCredentialManagerState["credentialStatus"] | undefined
): { label: string; tone: "normal" | "warning" | "danger" } {
  switch (status) {
    case "verified":
      return { label: "已验证", tone: "normal" };
    case "invalid":
      return { label: "凭证无效或已过期", tone: "danger" };
    case "invalid_context":
      return { label: "凭证归属不完整", tone: "danger" };
    case "unverified":
      return { label: "暂时无法在线校验", tone: "warning" };
    default:
      return { label: "尚未配置", tone: "warning" };
  }
}

function formatLocalAppLifecycle(
  state: LocalAppLifecycleState,
  detail?: string
): LocalAppLifecycle {
  const labels: Record<LocalAppLifecycleState, string> = {
    installed: "已安装",
    ready: "可用",
    missing: "未安装",
    broken: "需修复",
    updating: "更新中",
    starting: "启动中",
    running: "运行中",
    start_failed: "启动失败",
    stopped: "已停止",
    stopping: "停止中",
    unknown: "状态未知"
  };
  const details: Record<LocalAppLifecycleState, string> = {
    installed: "已安装，等待手动启动",
    ready: "工具已安装并通过校验",
    missing: "工具尚未安装",
    broken: "工具安装损坏或校验失败",
    updating: "正在安装或切换版本",
    starting: "正在执行应用启动命令",
    running: "应用正在运行",
    start_failed: "应用启动失败",
    stopped: "应用已停止",
    stopping: "正在执行应用停止命令",
    unknown: "等待运行状态检查"
  };
  const statusClasses: Record<LocalAppLifecycleState, string> = {
    installed: "installed",
    ready: "running",
    missing: "stopped",
    broken: "start_failed",
    updating: "starting",
    starting: "starting",
    running: "running",
    start_failed: "start_failed",
    stopped: "stopped",
    stopping: "stopping",
    unknown: "unknown"
  };
  return {
    state,
    label: labels[state],
    detail: detail ?? details[state],
    statusClass: statusClasses[state]
  };
}

function compareVersions(left: string, right: string): number {
  const parse = (value: string) => {
    const [core, prerelease = ""] = value.trim().replace(/^v/, "").split("-", 2);
    const parts = core.split(".").map((part) => Number.parseInt(part, 10) || 0);
    return { parts, prerelease };
  };
  const leftVersion = parse(left);
  const rightVersion = parse(right);
  const length = Math.max(leftVersion.parts.length, rightVersion.parts.length);
  for (let index = 0; index < length; index += 1) {
    const difference = (leftVersion.parts[index] ?? 0) - (rightVersion.parts[index] ?? 0);
    if (difference !== 0) {
      return difference > 0 ? 1 : -1;
    }
  }
  if (leftVersion.prerelease === rightVersion.prerelease) {
    return 0;
  }
  if (!leftVersion.prerelease) {
    return 1;
  }
  if (!rightVersion.prerelease) {
    return -1;
  }
  return leftVersion.prerelease.localeCompare(rightVersion.prerelease);
}

function formatConnectorServiceFailures(services: ConnectorServiceStartResult[]): string {
  return services
    .map((service) => `${service.service}${service.stderr.trim() ? ` ${service.stderr.trim()}` : ""}`)
    .join("；");
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
  if (hasSmartJsonQuotes(text)) {
    throw new Error('JSON 格式错误：请使用英文双引号 "，不要使用中文/智能引号 “ ”。');
  }
  return JSON.parse(text);
}

function prettyJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function hasSmartJsonQuotes(text: string): boolean {
  return /[“”]/.test(text);
}

function serviceSignature(service: UiServiceConfig): string {
  return JSON.stringify(service);
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

function formatRelayRegistration(snapshot: RuntimeSnapshot | null): string {
  if (!snapshot) {
    return "-";
  }
  if (!snapshot.relay_registered) {
    return "未注册";
  }
  return snapshot.relay_registered_at
    ? `已注册 ${formatEpochSeconds(snapshot.relay_registered_at)}`
    : "已注册";
}

function formatRelaySeen(snapshot: RuntimeSnapshot | null): string {
  if (!snapshot?.last_relay_seen_at) {
    return "-";
  }
  return formatTime(snapshot.last_relay_seen_at);
}

function formatEpochSeconds(timestamp: number): string {
  return formatTime(timestamp * 1000);
}

function calculateAppUpdateProgressPercent(progress: AppUpdateProgress | null): number | null {
  if (progress?.downloadedBytes == null || !progress.totalBytes || progress.totalBytes <= 0) {
    if (progress?.phase === "ready_to_install") {
      return 100;
    }
    return null;
  }
  return Math.max(0, Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100)));
}

function formatAppUpdateProgressButton(progress: AppUpdateProgress | null): string {
  const percent = calculateAppUpdateProgressPercent(progress);
  if (percent != null && progress?.phase === "downloading") {
    return `下载中 ${percent}%`;
  }
  switch (progress?.phase) {
    case "checking":
      return "检查中";
    case "verifying":
      return "校验中";
    case "saving":
      return "保存中";
    case "scheduling":
      return "准备安装";
    case "ready_to_install":
      return "即将重启";
    default:
      return "准备更新";
  }
}

function formatAppUpdateProgressDetail(progress: AppUpdateProgress): string {
  const sizeText =
    progress.downloadedBytes == null
      ? null
      : progress.totalBytes
        ? `${formatByteSize(progress.downloadedBytes)} / ${formatByteSize(progress.totalBytes)}`
        : `${formatByteSize(progress.downloadedBytes)} 已下载`;
  const parts = [
    progress.assetName ? `更新包 ${progress.assetName}` : null,
    sizeText,
    progress.downloadedPath && progress.phase !== "downloading" ? `保存到 ${progress.downloadedPath}` : null
  ].filter((part): part is string => Boolean(part));
  return parts.length > 0 ? parts.join("，") : "正在连接更新服务，请稍候。";
}

function formatByteSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value >= 10 || unitIndex === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unitIndex]}`;
}

function needsBrowserAuthorization(config: UiAgentConfig): boolean {
  return !config.platform.workspace_id.trim() || !config.relay.token.trim();
}

function formatStartAgentMessage(snapshot: RuntimeSnapshot): string {
  const messages: Partial<Record<RuntimeStatus, string>> = {
    starting: "Agent 正在启动",
    connecting: "Agent 正在连接",
    backoff: "Agent 正在重连等待",
    online: "Agent 已启动"
  };
  return messages[snapshot.status] ?? "Agent 已启动";
}

function readError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (isCommandError(error)) {
    if (error.code === "runtime_already_running") {
      return `百积木已经在运行，PID ${error.conflict.pid} 正在占用当前配置。`;
    }
    return error.message;
  }
  return String(error);
}

function readRuntimeConflict(error: unknown): RuntimeLockConflict | null {
  if (isCommandError(error) && error.code === "runtime_already_running") {
    return error.conflict;
  }
  return null;
}

function isCommandError(error: unknown): error is CommandError {
  if (!error || typeof error !== "object" || !("code" in error)) {
    return false;
  }
  const candidate = error as { code?: unknown; conflict?: unknown; message?: unknown };
  if (candidate.code === "message") {
    return typeof candidate.message === "string";
  }
  if (candidate.code !== "runtime_already_running") {
    return false;
  }
  const conflict = candidate.conflict as Partial<RuntimeLockConflict> | undefined;
  return Boolean(
    conflict &&
      typeof conflict.pid === "number" &&
      typeof conflict.agent_id === "string" &&
      typeof conflict.config_path === "string" &&
      typeof conflict.lock_path === "string" &&
      conflict.process &&
      typeof conflict.process === "object"
  );
}

export default App;
