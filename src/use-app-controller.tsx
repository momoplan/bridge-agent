import { invoke } from "@tauri-apps/api/core";


import { useMemo } from "react";
import { createLocalAppSelectors } from "./app/local-app-selectors";
import { createLocalAppUpdateSelectors } from "./app/local-app-update-selectors";
import { createLocalAppLifecycleActions } from "./app/local-app-lifecycle-actions";
import { createLocalAppCatalogActions } from "./app/local-app-catalog-actions";
import { useInterfaceEffects } from "./app/use-interface-effects";
import { usePlatformEffects } from "./app/use-platform-effects";
import { createDesktopActions } from "./app/desktop-actions";
import { createConfigEditorActions } from "./app/config-editor-actions";
import { useAppState } from "./app/use-app-state";


import { clientWarn } from "./client-logger";



import { deriveDeviceAuthorizationState, deviceAuthorizationLocksCapabilities } from "./device-authorization-state";

import { latestLocalAppInstallTasks, shouldShowLocalAppInstallTask } from "./local-app-install-tasks";

import { loadSynchronizedLocalAppCatalog } from "./local-app-catalog";








import { toUiConfig, isComputerService, isShellService, reindexRecordAfterDelete } from "./app/config-conversion";
import { serviceSignature, calculateAppUpdateProgressPercent, readError, readRuntimeConflict } from "./app/formatters";








import type { ConfigDocument, ConnectorLifecycleSnapshot, ConnectorSummary, DesktopPermissionStatus, LocalAppItem, LocalAppRuntimeStatus, LocalAppUpdateStatus, PythonRuntimeStatus, RegisteredServiceStatus, RuntimeSnapshot, RuntimeStatus, StartupHealthSnapshot } from "./app/types";



export function useAppController() {
  const appState = useAppState();
  const {
    setConfigPath, setManifestPreview, config, setConfig, runtime, setRuntime, logs,
    logServiceFilter, setRefreshing, setMessage, setError, setRuntimeConflict, browserAuth,
    appVersion, appUpdate, desktopPermissions, setDesktopPermissions, setRegisteredServiceStatuses,
    setLocalAppRuntimeStatuses, connectorApps, setConnectorApps, localAppInstallTasks,
    localAppsChangeRevisionRef, marketConnectors, baijimuCli, appUpdateCheckState,
    appUpdateError, appUpdateProgress, setStartupHealth, setConnectorLifecycles,
    setSavedServiceSignatures, setServiceNotices, setServiceJsonDrafts,
    setServiceJsonErrors, setCapabilityTestDrafts, setCapabilityTestResults,
    expandedMethodAdvancedKey, setExpandedMethodAdvancedKey, setActivePage,
    setExpandedServiceIndex, selectedLocalAppId, pendingUpgradeAppId, marketAppQuery,
    pythonStatus, setPythonStatus, setPythonCheckBusy,
  } = appState;

  const installableMarketConnectors = useMemo(
    () => marketConnectors.filter((app) => app.applicationType !== "managed_tool"),
    [marketConnectors]
  );

  const visibleMarketConnectors = useMemo(() => {
    const query = marketAppQuery.trim().toLocaleLowerCase();
    if (!query) {
      return installableMarketConnectors;
    }
    return installableMarketConnectors.filter((app) =>
      [app.name, app.description, app.capability, app.version].some((value) =>
        value.toLocaleLowerCase().includes(query)
      )
    );
  }, [installableMarketConnectors, marketAppQuery]);

  const authorizationState = useMemo(() => deriveDeviceAuthorizationState({
    workspaceId: config?.platform.workspace_id ?? "",
    relayTokenConfigured: config?.credential_status.relay_token_configured ?? false,
    runtimeStatus: runtime?.status,
    authorizationPending: browserAuth != null
  }), [browserAuth, config, runtime?.status]);
  const needsAuthorization = deviceAuthorizationLocksCapabilities(authorizationState);

  const statusLabel = useMemo(() => {
    if (authorizationState === "unauthorized") {
      return "未授权";
    }
    if (authorizationState === "authorizing") {
      return "授权中";
    }
    if (authorizationState === "reauthorization_required") {
      return "需重新授权";
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
      authorization_required: "需重新授权",
      stopping: "停止中"
    };
    return textMap[runtime.status];
  }, [authorizationState, runtime]);

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
    const latestInstallTasks = latestLocalAppInstallTasks(localAppInstallTasks);
    if (!config) {
      return [];
    }
    const apps: LocalAppItem[] = connectorApps.map((connector) => {
      const localAppIndex = config.local_apps.findIndex(
        (localApp) => localApp.appId === connector.appId
      );
      const capabilityCount =
        connector.methodNames.length + connector.eventNames.length;
      const latestInstallTask = latestInstallTasks
        .filter(
          (task) =>
            task.appId === connector.appId
        )
        .sort((left, right) => right.updatedAtEpochMs - left.updatedAtEpochMs)[0];
      const installTask = latestInstallTask?.phase === "succeeded" ? undefined : latestInstallTask;
      return {
        id: `connector:${connector.appId}`,
        name: connector.name,
        description: `版本 ${connector.version} · ${capabilityCount} 项能力`,
        kind: "connector",
        serviceIndexes: [],
        localAppIndex: localAppIndex >= 0 ? localAppIndex : undefined,
        connector,
        installTask
      };
    });

    latestInstallTasks.forEach((installTask) => {
      if (
        !shouldShowLocalAppInstallTask(
          installTask,
          connectorApps.map((connector) => connector.appId)
        )
      ) {
        return;
      }
      apps.push({
        id: `install-task:${installTask.taskId}`,
        name: installTask.name,
        description: [installTask.version ? `版本 ${installTask.version}` : null, installTask.message]
          .filter(Boolean)
          .join(" · "),
        kind: "connector",
        serviceIndexes: [],
        installTask
      });
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
  }, [config, connectorApps, baijimuCli, localAppInstallTasks]);
  const selectedLocalApp =
    selectedLocalAppId == null ? null : localApps.find((app) => app.id === selectedLocalAppId) ?? null;
  const pendingUpgradeApp =
    pendingUpgradeAppId == null ? null : localApps.find((app) => app.id === pendingUpgradeAppId) ?? null;
  const { localAppUpdateStatus } = createLocalAppUpdateSelectors(marketConnectors);
  const availableLocalAppUpdates = localApps
    .map((app) => ({ app, status: localAppUpdateStatus(app) }))
    .filter(
      (item): item is { app: LocalAppItem; status: LocalAppUpdateStatus } =>
        item.status?.updateAvailable === true
    );
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
  const appUpdateTone: "normal" | "warning" | "danger" =
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
    applyRuntimeSnapshot(document.runtime);
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
    applyRuntimeSnapshot(document.runtime);
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
    applyRuntimeSnapshot(document.runtime);
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
      setActivePage("apps");
      return;
    }
    setError(readError(err));
  }

  async function refreshAll() {
    setRefreshing(true);
    try {
      setError("");
      setRuntimeConflict(null);
      const [{ apps, document }] = await Promise.all([
        loadSynchronizedLocalAppCatalog(
          () => invoke<ConnectorSummary[]>("list_connector_apps"),
          () => invoke<ConfigDocument>("load_config")
        ),
        loadAppVersion()
      ]);
      setConnectorApps(apps);
      applyConfigDocument(document);
      await refreshLocalAppUpdateData();
      await refreshRegisteredServiceStatuses();
    } catch (err) {
      handleCommandError(err);
    } finally {
      setRefreshing(false);
    }
  }

  async function refreshPythonRuntime(pythonPath?: string | null) {
    try {
      setPythonCheckBusy(true);
      const status = await invoke<PythonRuntimeStatus>("python_runtime_status", {
        pythonPath: pythonPath ?? ""
      });
      setPythonStatus(status);
    } catch (err) {
      setPythonStatus({
        available: false,
        message: readError(err)
      });
    } finally {
      setPythonCheckBusy(false);
    }
  }

  function useDetectedPython() {
    if (!pythonStatus?.available || !pythonStatus.detectedPath) {
      return;
    }
    updateRuntime("python_path", pythonStatus.detectedPath);
    setMessage("已填入检测到的 Python 路径，请保存配置。");
  }

  async function refreshLocalAppCatalog(revision: number) {
    try {
      const { apps, document } = await loadSynchronizedLocalAppCatalog(
        () => invoke<ConnectorSummary[]>("list_connector_apps"),
        () => invoke<ConfigDocument>("load_config")
      );
      if (revision < localAppsChangeRevisionRef.current) {
        return;
      }
      setConnectorApps(apps);
      applyConfigDocument(document);
      await refreshRegisteredServiceStatuses();
    } catch (err) {
      clientWarn("刷新本地应用目录失败", err);
    }
  }

  function applyStartupHealthSnapshot(snapshot: StartupHealthSnapshot) {
    setStartupHealth((current) =>
      current == null || snapshot.revision >= current.revision ? snapshot : current
    );
  }

  function applyRuntimeSnapshot(snapshot: RuntimeSnapshot) {
    setRuntime((current) =>
      current == null || snapshot.revision >= current.revision ? snapshot : current
    );
  }

  async function refreshRuntime() {
    try {
      const snapshot = await invoke<RuntimeSnapshot>("runtime_snapshot");
      applyRuntimeSnapshot(snapshot);
    } catch (err) {
      setError(readError(err));
    }
  }

  async function refreshDesktopPermissions() {
    try {
      const status = await invoke<DesktopPermissionStatus>("desktop_permission_status");
      setDesktopPermissions(status);
    } catch (err) {
      clientWarn("读取桌面权限状态失败", err);
    }
  }

  async function refreshRegisteredServiceStatuses() {
    try {
      const [serviceStatuses, appStatuses] = await Promise.all([
        invoke<RegisteredServiceStatus[]>("registered_service_statuses"),
        invoke<LocalAppRuntimeStatus[]>("local_app_runtime_statuses")
      ]);
      const lifecycleSnapshots = await invoke<ConnectorLifecycleSnapshot[]>(
        "connector_lifecycle_snapshots"
      );
      setRegisteredServiceStatuses(serviceStatuses);
      setLocalAppRuntimeStatuses(appStatuses);
      setConnectorLifecycles(
        Object.fromEntries(lifecycleSnapshots.map((snapshot) => [snapshot.appId, snapshot]))
      );
      return appStatuses;
    } catch (err) {
      clientWarn("读取本地应用运行状态失败", err);
      return null;
    }
  }

  const localAppCatalogActions = createLocalAppCatalogActions(appState, { applyRuntimeSnapshot, formatApplyMessage, handleCommandError, installableMarketConnectors, refreshRegisteredServiceStatuses });
  const { refreshConnectorApps, refreshLocalAppUpdateData, startRegisteredService, stopRegisteredService, startLocalAppInstallTask } = localAppCatalogActions;

  const localAppSelectors = createLocalAppSelectors(appState);
  const { hasLocalAppStartCommand, hasLocalAppStopCommand, setLocalAppLifecycleOverride, clearLocalAppLifecycleOverride } = localAppSelectors;
  const localAppLifecycleActions = createLocalAppLifecycleActions(appState, { applyConfigDocument, applyRuntimeSnapshot, buildCapabilityTestKey, clearLocalAppLifecycleOverride, formatApplyMessage, handleCommandError, hasLocalAppStartCommand, hasLocalAppStopCommand, refreshConnectorApps, refreshLocalAppUpdateData, refreshRegisteredServiceStatuses, refreshRuntime, setLocalAppLifecycleOverride, startLocalAppInstallTask, startRegisteredService, stopRegisteredService });

  const desktopActions = createDesktopActions(appState, { appUpdateProgressPercent, applyConfigDocument, applyDeletedServiceDocument, applyRuntimeSnapshot, applySavedServiceDocument, formatApplyMessage, handleCommandError, refreshRegisteredServiceStatuses, refreshRuntime });
  const { loadAppVersion } = desktopActions;

  const configEditorActions = createConfigEditorActions(appState);
  const { updateRuntime } = configEditorActions;


  const effectContext = { ...appState, ...localAppCatalogActions, ...desktopActions, applyRuntimeSnapshot, applyStartupHealthSnapshot, refreshAll, refreshDesktopPermissions, refreshLocalAppCatalog, authorizationState, localApps, needsAuthorization, selectedLocalApp, visibleMarketConnectors };
  usePlatformEffects(effectContext);
  useInterfaceEffects(effectContext);

  return {
    ...appState,
    ...localAppCatalogActions,
    ...localAppLifecycleActions,
    ...desktopActions,
    ...configEditorActions,
    ...localAppSelectors,
    installableMarketConnectors,
    visibleMarketConnectors,
    authorizationState,
    needsAuthorization,
    statusLabel,
    startActionLocked,
    runtimeCanStop,
    startActionLabel,
    latestLog,
    logServiceOptions,
    filteredLogs,
    enabledComputerMethodCount,
    localApps,
    selectedLocalApp,
    pendingUpgradeApp,
    availableLocalAppUpdates,
    appVersionLabel,
    forceUpdateRequired,
    appUpdateStatusLabel,
    appUpdateTone,
    appUpdateProgressPercent,
    hasDesktopPermissionGap,
    buildMethodEditorKey,
    buildCapabilityTestKey,
    isMethodAdvancedOpen,
    toggleMethodAdvanced,
    openLocalAppCapabilityConfig,
    applyConfigDocument,
    applySavedServiceDocument,
    applyDeletedServiceDocument,
    formatApplyMessage,
    handleCommandError,
    refreshAll,
    refreshPythonRuntime,
    useDetectedPython,
    refreshLocalAppCatalog,
    applyStartupHealthSnapshot,
    applyRuntimeSnapshot,
    refreshRuntime,
    refreshDesktopPermissions,
    refreshRegisteredServiceStatuses,
  };
}
