import type { ConnectorLifecycleSnapshot } from "./apps";

export type RuntimeStatus =
  | "stopped"
  | "starting"
  | "connecting"
  | "online"
  | "backoff"
  | "authorization_required"
  | "stopping";

export interface RuntimeSnapshot {
  revision: number;
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

export interface RuntimeProcessInfo {
  pid: number;
  parent_pid: number | null;
  name: string | null;
  executable_path: string | null;
  command_line: string | null;
  running: boolean;
}

export interface RuntimeLockConflict {
  pid: number;
  agent_id: string;
  config_path: string;
  lock_path: string;
  process: RuntimeProcessInfo;
}

export type CommandError =
  | { code: "runtime_already_running"; conflict: RuntimeLockConflict }
  | { code: "connector_uninstall_stop_failed"; message: string }
  | { code: "connector_uninstall_failed"; message: string }
  | { code: "connector_not_ready"; message: string; lifecycle: ConnectorLifecycleSnapshot }
  | { code: "connector_management_failed"; message: string }
  | { code: "message"; message: string };

export interface LogEntry {
  sequence: number;
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

export interface RelayConfig {
  url: string;
  agent_id: string;
  token: string;
  token_issued_at_epoch_seconds?: string | null;
  token_expires_at_epoch_seconds?: string | null;
  reconnect_secs: number;
}

export interface PlatformConfig {
  base_url: string;
  environment_key?: string | null;
  workspace_id: number | null;
}

export interface UploadConfig {
  prepare_url?: string | null;
  inline_limit_bytes: number;
  timeout_secs: number;
}

export interface DeviceConfig {
  name: string;
  description: string;
  tags: string[];
}

export interface RuntimeConfig {
  node_path?: string | null;
  python_path?: string | null;
  default_timeout_secs: number;
  max_timeout_secs: number;
  log_limit: number;
  log_file_enabled: boolean;
  log_file_dir?: string | null;
  log_file_max_bytes: number;
  log_file_max_files: number;
  event_server_enabled: boolean;
  event_server_bind: string;
  service_registration_enabled: boolean;
  service_registration_token?: string | null;
}

export interface PythonRuntimeStatus {
  configuredPath?: string | null;
  detectedPath?: string | null;
  version?: string | null;
  available: boolean;
  message: string;
}

export interface ShellBinding {
  type: "shell_command";
  root_dir: string;
  allow_commands: string[];
  default_timeout_secs?: number | null;
  max_timeout_secs?: number | null;
}

export interface HttpBinding {
  type: "http";
  url: string;
  http_method: string;
  headers: Record<string, string>;
  timeout_secs?: number | null;
}

export type ComputerAction =
  | "screenshot"
  | "click"
  | "double_click"
  | "scroll"
  | "type"
  | "wait"
  | "keypress"
  | "drag"
  | "move";

export interface ComputerUseBinding {
  type: "computer_use";
  action: ComputerAction;
  display_id?: number | null;
}

export type MethodBinding = ShellBinding | HttpBinding | ComputerUseBinding;

export interface MethodConfig {
  name: string;
  description: string;
  enabled: boolean;
  input_schema: unknown;
  binding: MethodBinding;
}

export interface EventConfig {
  name: string;
  description: string;
  enabled: boolean;
  payload_schema: unknown;
}

export interface ServiceHealthCheckHttp {
  type: "http";
  url: string;
  http_method: string;
  headers: Record<string, string>;
  timeout_secs?: number | null;
  expect_status?: number | null;
  body_contains?: string | null;
}

export type ServiceHealthCheck = ServiceHealthCheckHttp;

export interface ServiceStartShellCommand {
  type: "shell_command";
  command: string[];
  cwd?: string | null;
  env: Record<string, string>;
  timeout_secs?: number | null;
}

export type ServiceStartCommand = ServiceStartShellCommand;

export interface ServiceConfig {
  name: string;
  description: string;
  enabled: boolean;
  health_check?: ServiceHealthCheck | null;
  start_command?: ServiceStartCommand | null;
  stop_command?: ServiceStartCommand | null;
  methods: MethodConfig[];
}

export interface AgentConfig {
  platform: PlatformConfig;
  upload: UploadConfig;
  relay: RelayConfig;
  device: DeviceConfig;
  runtime: RuntimeConfig;
  services: ServiceConfig[];
  local_apps: LocalAppConfig[];
  credential_status?: {
    relay_token_configured: boolean;
  };
}

export interface LocalAppConfig {
  appId: string;
  name: string;
  version: string;
  description: string;
  enabled: boolean;
  healthCheck?: ServiceHealthCheck | null;
  startCommand?: ServiceStartCommand | null;
  stopCommand?: ServiceStartCommand | null;
  methods: MethodConfig[];
  events: EventConfig[];
}

export interface BrowserAuthStartResponse {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string;
  expiresIn: number;
  interval: number;
}

export interface BrowserAuthPollResponse {
  status: "pending" | "authorized" | "denied" | "expired";
  message: string;
  config: AgentConfig | null;
  runtime: RuntimeSnapshot | null;
}

export interface CapabilityInvokeError {
  code: string;
  message: string;
}

export interface CapabilityInvokeResult {
  request_id: string;
  success: boolean;
  data?: unknown | null;
  error?: CapabilityInvokeError | null;
  duration_ms: number;
}

export interface CapabilityTestState {
  status: "success" | "error";
  result?: CapabilityInvokeResult;
  message?: string;
}

export interface ConfigDocument {
  config_path: string;
  manifest_preview: string;
  config: AgentConfig;
  runtime: RuntimeSnapshot;
}

export interface ConfigRecoveryDocument extends ConfigDocument {
  archived_path: string | null;
}

export interface AppUpdateStatus {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  forceUpdateRequired: boolean;
  minimumSupportedVersion: string | null;
  forceUpdateMessage: string | null;
  releaseUrl: string | null;
  releaseName: string | null;
  publishedAt: string | null;
  currentTarget: string;
  autoDownloadAvailable: boolean;
  assetName: string | null;
}

export interface AppVersionInfo {
  currentVersion: string;
  currentTarget: string;
}

export type AppUpdateCheckState = "checking" | "ready" | "error";

export interface AppUpdateInstallResult {
  status: "up_to_date" | "installed";
  version: string;
  assetName: string | null;
  downloadedPath: string | null;
}

export type AppUpdateProgressPhase =
  | "checking"
  | "downloading"
  | "installing"
  | "verifying"
  | "saving"
  | "scheduling"
  | "ready_to_install";

export interface AppUpdateProgress {
  phase: AppUpdateProgressPhase;
  message: string;
  version: string | null;
  assetName: string | null;
  downloadedBytes: number | null;
  totalBytes: number | null;
  downloadedPath: string | null;
}

export interface StartupComponentHealth {
  id: string;
  label: string;
  status: "starting" | "ready" | "unavailable" | "degraded" | "skipped" | string;
  detail: string | null;
}

export interface StartupHealthSnapshot {
  revision: number;
  safeMode: boolean;
  forcedSafeMode: boolean;
  consecutiveFailures: number;
  frontendReady: boolean;
  startupLogPath: string;
  components: StartupComponentHealth[];
}
