import type { InstallSource } from "../market-identity";
import type { ComputerAction, LocalAppConfig, MethodConfig, RelayConfig, RuntimeConfig } from "./runtime";
import type { UpdateDatabaseContract, UpdateEventContract, UpdateMethodContract } from "../../local-app-updates";

export interface DesktopPermissionStatus {
  platform: string;
  accessibilityGranted: boolean;
  screenRecordingGranted: boolean;
  accessibilitySupported: boolean;
  screenRecordingSupported: boolean;
}

export interface UiShellBinding {
  type: "shell_command";
  root_dir: string;
  allow_commands_text: string;
  default_timeout_secs: string;
  max_timeout_secs: string;
}

export interface UiHttpBinding {
  type: "http";
  url: string;
  http_method: string;
  headers_text: string;
  timeout_secs: string;
}

export interface UiComputerUseBinding {
  type: "computer_use";
  action: ComputerAction;
  display_id: string;
}

export type UiMethodBinding = UiShellBinding | UiHttpBinding | UiComputerUseBinding;

export interface UiMethodConfig {
  name: string;
  description: string;
  enabled: boolean;
  input_schema_text: string;
  binding: UiMethodBinding;
}

export interface UiEventConfig {
  name: string;
  description: string;
  enabled: boolean;
  payload_schema_text: string;
}

export interface UiServiceHealthCheckHttp {
  type: "http";
  url: string;
  http_method: string;
  headers_text: string;
  timeout_secs: string;
  expect_status: string;
  body_contains: string;
}

export type UiServiceHealthCheck = UiServiceHealthCheckHttp;

export interface UiServiceStartShellCommand {
  type: "shell_command";
  command_text: string;
  cwd: string;
  env_text: string;
  timeout_secs: string;
}

export type UiServiceStartCommand = UiServiceStartShellCommand;

export interface UiServiceConfig {
  name: string;
  description: string;
  enabled: boolean;
  health_check: UiServiceHealthCheck | null;
  start_command: UiServiceStartCommand | null;
  stop_command: UiServiceStartCommand | null;
  methods: UiMethodConfig[];
}

export interface ServiceCapabilitiesDocument {
  methods: MethodConfig[];
}

export type RegisteredServiceState = "not_configured" | "healthy" | "unhealthy" | "unknown";

export interface RegisteredServiceStatus {
  service: string;
  status: RegisteredServiceState;
  detail: string | null;
  checkedAtMs: number;
  healthCheckConfigured: boolean;
  startCommandConfigured: boolean;
  stopCommandConfigured: boolean;
}

export interface LocalAppRuntimeStatus {
  appId: string;
  status: RegisteredServiceState;
  detail: string | null;
  checkedAtMs: number;
  healthCheckConfigured: boolean;
  startCommandConfigured: boolean;
  stopCommandConfigured: boolean;
  processManaged: boolean;
  processRunning: boolean | null;
}

export interface StartRegisteredServiceResult {
  service: string;
  success: boolean;
  exitCode: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

export interface ConnectorSummary {
  installSource?: InstallSource | null;
  appId: string;
  name: string;
  version: string;
  packagePath: string;
  sourcePath: string;
  sourceReference?: string | null;
  reviewStatus: string;
  sourceChecksum?: string | null;
  packageChecksum?: string | null;
  iconDataUrl?: string | null;
  ui?: ConnectorUi | null;
  permissions: ConnectorPermission[];
  startPolicy: "automatic" | "manual";
  processOwnership: "connector" | "host";
  configSchema?: unknown | null;
  database?: UpdateDatabaseContract | null;
  methods?: UpdateMethodContract[];
  events?: UpdateEventContract[];
  methodNames: string[];
  eventNames: string[];
  installedAtEpochMs: number;
  lastSyncedAtEpochMs: number;
}

export interface LocalAppsChangedEvent {
  revision: number;
  operation: "install" | "upgrade" | "sync" | "uninstall";
  appId: string;
}

export interface ConnectorPermission {
  id: string;
  title: string;
  description: string;
  platforms: string[];
}

export interface ConnectorUi {
  type: "embedded";
  entry: string;
  title?: string | null;
  defaultView: boolean;
}

export interface ConnectorLifecycleResult {
  appId: string;
  configured: boolean;
  exitCode: number | null;
  stdout: string;
  stderr: string;
}

export interface ConnectorStartResult {
  appId: string;
  lifecycle: ConnectorLifecycleResult;
}

export interface ConnectorAppUpdateStatus {
  appId: string;
  name: string;
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  source: string;
}

export interface LocalAppUpdateStatus {
  appId: string;
  name: string;
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  source: string;
}

export interface UiAgentConfig {
  platform: {
    base_url: string;
    environment_key?: string | null;
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
  local_apps: LocalAppConfig[];
  credential_status: {
    relay_token_configured: boolean;
  };
}
