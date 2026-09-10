import type { InstallSource } from "../market-identity";
import type { DesktopPage } from "../../components/DesktopShell";
import type { LocalAppInstallTask, LocalAppInstallTaskOperation } from "../../local-app-install-tasks";
import type { UpdateContractDeclaration, UpdateDatabaseContract, UpdateEventContract, UpdateMethodContract } from "../../local-app-updates";
import type { ConnectorPermission, ConnectorSummary } from "./config";
export type SettingsSection = "identity" | "connection" | "runtime";
export type AppPage = DesktopPage;
export type DetailPanel = "system" | "settings" | "logs" | "manifest";
export type LocalAppKind = "connector" | "managed_tool" | "built_in" | "custom";
export type InstallSourceMode = "market" | "custom";
export type LocalAppDetailTab = "app" | "overview" | "capabilities" | "config";
export type LocalAppLifecycleState =
  | "absent"
  | "installing"
  | "stopped"
  | "starting"
  | "ready"
  | "degraded"
  | "stopping"
  | "upgrading"
  | "uninstalling"
  | "recovering"
  | "failed";

export type ConnectorHealthState = "not_configured" | "healthy" | "unhealthy" | "unknown";
export type ConnectorOperationKind = "install" | "upgrade" | "start" | "stop" | "uninstall";

export interface ConnectorOperationSnapshot {
  id: string;
  kind: ConnectorOperationKind;
  phase: string;
  progressPercent: number | null;
  startedAtEpochMs: number;
  updatedAtEpochMs: number;
}

export interface ConnectorLifecycleSnapshot {
  schemaVersion: number;
  appId: string;
  lifecycle: LocalAppLifecycleState;
  operation: ConnectorOperationSnapshot | null;
  health: ConnectorHealthState;
  desiredVersion: string | null;
  observedVersion: string | null;
  desiredGeneration: number;
  observedGeneration: number;
  pid: number | null;
  detail: string | null;
  error: string | null;
  updatedAtEpochMs: number;
}

export interface LocalAppLifecycleOverride {
  state: LocalAppLifecycleState;
  detail?: string;
}

export interface LocalAppLifecycle {
  state: LocalAppLifecycleState;
  label: string;
  detail: string;
  statusClass: string;
}

export interface LocalAppItem {
  id: string;
  name: string;
  description: string;
  kind: LocalAppKind;
  serviceIndexes: number[];
  localAppIndex?: number;
  connector?: ConnectorSummary;
  managedTool?: ManagedToolStatus;
  installTask?: LocalAppInstallTask;
}

export interface StartConnectorAppInstallRequest {
  installSource?: InstallSource | null;
  operation: LocalAppInstallTaskOperation;
  replace: boolean;
  appId: string;
  name: string | null;
  version: string;
  acceptUnreviewed: boolean;
}

export interface MarketConnector {
  installSource?: InstallSource | null;
  appId: string;
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
  publishedAt?: string | null;
  iconDataUrl?: string | null;
  releaseNotes: string[];
  configurationDeclaration: UpdateContractDeclaration;
  interfaceDeclaration: UpdateContractDeclaration;
  databaseDeclaration: UpdateContractDeclaration;
  configSchema: unknown | null;
  database: UpdateDatabaseContract | null;
  methods: UpdateMethodContract[];
  events: UpdateEventContract[];
  methodNames: string[];
  eventNames: string[];
  permissions: ConnectorPermission[];
  compatible: boolean;
  compatibilityMessage?: string | null;
  minimumHostVersion?: string | null;
  requiredHostCapabilities: string[];
  missingHostCapabilities: string[];
}

export interface ManagedToolStatus {
  installSource?: InstallSource | null;
  id: string;
  name: string;
  description: string;
  state: "ready" | "missing" | "broken";
  installedVersion?: string | null;
  bundledVersion?: string | null;
  previousVersion?: string | null;
  activePath: string;
  launcherPath: string;
  pathConfigured: boolean;
  restartRequired: boolean;
  canRollback: boolean;
  detail: string;
}
