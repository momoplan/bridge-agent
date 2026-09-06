import { useRef, useState } from "react";
import { deriveDeviceAuthorizationState } from "../device-authorization-state";
import type { BaijimuDeepLinkIntent } from "../deep-link";
import type { LocalAppInstallTask } from "../local-app-install-tasks";
import { configurationStartupFailureDetail, configurationStartupIsAllowed, type StartupUpdateGateState } from "../startup-update-gate";
import type { AppPage, AppUpdateCheckState, AppUpdateProgress, AppUpdateStatus, AppVersionInfo, BrowserAuthStartResponse, CapabilityTestState, ConnectorAppUpdateStatus, ConnectorLifecycleSnapshot, ConnectorSummary, DesktopPermissionStatus, DetailPanel, InstallSourceMode, LocalAppDetailTab, LocalAppKind, LocalAppLifecycleOverride, LocalAppRuntimeStatus, LogEntry, ManagedToolStatus, MarketConnector, PythonRuntimeStatus, RegisteredServiceStatus, RuntimeLockConflict, RuntimeSnapshot, SettingsSection, StartupHealthSnapshot, UiAgentConfig } from "./types";

export function useAppState() {
  const [configPath, setConfigPath] = useState("");
  const [manifestPreview, setManifestPreview] = useState("");
  const [config, setConfig] = useState<UiAgentConfig | null>(null);
  const [savedServiceSignatures, setSavedServiceSignatures] = useState<string[]>([]);
  const [runtime, setRuntime] = useState<RuntimeSnapshot | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const logsClearedThroughRef = useRef(0);
  const [mainWindowVisible, setMainWindowVisible] = useState(true);
  const [logServiceFilter, setLogServiceFilter] = useState("");
  const [localAppQuery, setLocalAppQuery] = useState("");
  const [localAppKindFilter, setLocalAppKindFilter] = useState<LocalAppKind | "all">("all");
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [runtimeConflict, setRuntimeConflict] = useState<RuntimeLockConflict | null>(null);
  const [browserAuth, setBrowserAuth] = useState<BrowserAuthStartResponse | null>(null);
  const [appVersion, setAppVersion] = useState<AppVersionInfo | null>(null);
  const [appUpdate, setAppUpdate] = useState<AppUpdateStatus | null>(null);
  const [desktopPermissions, setDesktopPermissions] = useState<DesktopPermissionStatus | null>(null);
  const [registeredServiceStatuses, setRegisteredServiceStatuses] = useState<RegisteredServiceStatus[]>([]);
  const [localAppRuntimeStatuses, setLocalAppRuntimeStatuses] = useState<LocalAppRuntimeStatus[]>([]);
  const [connectorApps, setConnectorApps] = useState<ConnectorSummary[]>([]);
  const [localAppInstallTasks, setLocalAppInstallTasks] = useState<LocalAppInstallTask[]>([]);
  const connectorAppsRef = useRef<ConnectorSummary[]>([]);
  const localAppInstallTasksRef = useRef<LocalAppInstallTask[]>([]);
  const localAppsChangeRevisionRef = useRef(0);
  const localAppUpdateRefreshRef = useRef<Promise<void> | null>(null);
  const previousActivePageRef = useRef<AppPage>("apps");
  const authorizationStateRef = useRef<ReturnType<typeof deriveDeviceAuthorizationState> | null>(null);
  const queuedDeepLinkIntentsRef = useRef<BaijimuDeepLinkIntent[]>([]);
  const [marketConnectors, setMarketConnectors] = useState<MarketConnector[]>([]);
  const [baijimuCli, setBaijimuCli] = useState<ManagedToolStatus | null>(null);
  const [connectorUpdateStatuses, setConnectorUpdateStatuses] = useState<Record<string, ConnectorAppUpdateStatus>>({});
  const [appUpdateCheckState, setAppUpdateCheckState] = useState<AppUpdateCheckState>("checking");
  const [appUpdateError, setAppUpdateError] = useState<string | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [appUpdateProgress, setAppUpdateProgress] = useState<AppUpdateProgress | null>(null);
  const [startupHealth, setStartupHealth] = useState<StartupHealthSnapshot | null>(null);
  const [startupRecoveryBusy, setStartupRecoveryBusy] = useState(false);
  const [serviceStartBusy, setServiceStartBusy] = useState<string | null>(null);
  const [connectorBusy, setConnectorBusy] = useState<string | null>(null);
  const [connectorUninstalling, setConnectorUninstalling] = useState<string | null>(null);
  const [connectorLifecycles, setConnectorLifecycles] = useState<
    Record<string, ConnectorLifecycleSnapshot>
  >({});
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
  const [activePage, setActivePage] = useState<AppPage>("apps");
  const [activeDetailPanel, setActiveDetailPanel] = useState<DetailPanel>("system");
  const [expandedServiceIndex, setExpandedServiceIndex] = useState<number | null>(0);
  const [selectedLocalAppId, setSelectedLocalAppId] = useState<string | null>(null);
  const [pendingUpgradeAppId, setPendingUpgradeAppId] = useState<string | null>(null);
  const [activeLocalAppDetailTab, setActiveLocalAppDetailTab] = useState<LocalAppDetailTab>("overview");
  const [installPanelOpen, setInstallPanelOpen] = useState(false);
  const [installSourceMode, setInstallSourceMode] = useState<InstallSourceMode>("market");
  const [selectedMarketAppId, setSelectedMarketAppId] = useState("");
  const [marketAppQuery, setMarketAppQuery] = useState("");
  const [marketLoading, setMarketLoading] = useState(false);
  const [marketLoadError, setMarketLoadError] = useState("");
  const [registeredInstallAppId, setRegisteredInstallAppId] = useState("");
  const [registeredInstallVersion, setRegisteredInstallVersion] = useState("");
  const [customInstallConfirmed, setCustomInstallConfirmed] = useState(false);
  const [installBusy, setInstallBusy] = useState(false);
  const [pythonStatus, setPythonStatus] = useState<PythonRuntimeStatus | null>(null);
  const [pythonCheckBusy, setPythonCheckBusy] = useState(false);
  const [startupUpdateGate, setStartupUpdateGate] =
    useState<StartupUpdateGateState>("checking");
  const startupConfigGateReady = configurationStartupIsAllowed(
    startupUpdateGate,
    startupHealth?.components
  );
  const startupConfigMigrationFailure = configurationStartupFailureDetail(
    startupHealth?.components
  );

  return { configPath, setConfigPath, manifestPreview, setManifestPreview, config, setConfig, savedServiceSignatures, setSavedServiceSignatures, runtime, setRuntime, logs, setLogs, logsClearedThroughRef, mainWindowVisible, setMainWindowVisible, logServiceFilter, setLogServiceFilter, localAppQuery, setLocalAppQuery, localAppKindFilter, setLocalAppKindFilter, refreshing, setRefreshing, busy, setBusy, message, setMessage, error, setError, runtimeConflict, setRuntimeConflict, browserAuth, setBrowserAuth, appVersion, setAppVersion, appUpdate, setAppUpdate, desktopPermissions, setDesktopPermissions, registeredServiceStatuses, setRegisteredServiceStatuses, localAppRuntimeStatuses, setLocalAppRuntimeStatuses, connectorApps, setConnectorApps, localAppInstallTasks, setLocalAppInstallTasks, connectorAppsRef, localAppInstallTasksRef, localAppsChangeRevisionRef, localAppUpdateRefreshRef, previousActivePageRef, authorizationStateRef, queuedDeepLinkIntentsRef, marketConnectors, setMarketConnectors, baijimuCli, setBaijimuCli, connectorUpdateStatuses, setConnectorUpdateStatuses, appUpdateCheckState, setAppUpdateCheckState, appUpdateError, setAppUpdateError, updateBusy, setUpdateBusy, appUpdateProgress, setAppUpdateProgress, startupHealth, setStartupHealth, startupRecoveryBusy, setStartupRecoveryBusy, serviceStartBusy, setServiceStartBusy, connectorBusy, setConnectorBusy, connectorUninstalling, setConnectorUninstalling, connectorLifecycles, setConnectorLifecycles, localAppLifecycleOverrides, setLocalAppLifecycleOverrides, connectorUpdateBusy, setConnectorUpdateBusy, managedToolBusy, setManagedToolBusy, serviceNotices, setServiceNotices, serviceJsonDrafts, setServiceJsonDrafts, serviceJsonErrors, setServiceJsonErrors, capabilityTestDrafts, setCapabilityTestDrafts, capabilityTestResults, setCapabilityTestResults, capabilityTestBusy, setCapabilityTestBusy, desktopPermissionBusy, setDesktopPermissionBusy, expandedMethodAdvancedKey, setExpandedMethodAdvancedKey, showAdvancedSettings, setShowAdvancedSettings, activeSettingsSection, setActiveSettingsSection, activePage, setActivePage, activeDetailPanel, setActiveDetailPanel, expandedServiceIndex, setExpandedServiceIndex, selectedLocalAppId, setSelectedLocalAppId, pendingUpgradeAppId, setPendingUpgradeAppId, activeLocalAppDetailTab, setActiveLocalAppDetailTab, installPanelOpen, setInstallPanelOpen, installSourceMode, setInstallSourceMode, selectedMarketAppId, setSelectedMarketAppId, marketAppQuery, setMarketAppQuery, marketLoading, setMarketLoading, marketLoadError, setMarketLoadError, registeredInstallAppId, setRegisteredInstallAppId, registeredInstallVersion, setRegisteredInstallVersion, customInstallConfirmed, setCustomInstallConfirmed, installBusy, setInstallBusy, pythonStatus, setPythonStatus, pythonCheckBusy, setPythonCheckBusy, startupUpdateGate, setStartupUpdateGate, startupConfigGateReady, startupConfigMigrationFailure };
}

export type AppControllerState = ReturnType<typeof useAppState>;
