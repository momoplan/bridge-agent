import { marketSelectionKey } from "./market-identity";
import { Channel, invoke } from "@tauri-apps/api/core";
import { clientInfo, clientWarn } from "../client-logger";
import type { BaijimuDeepLinkIntent, LocalAppInstallDeepLinkIntent } from "../deep-link";
import { legacyEventContracts, legacyMethodContracts } from "./legacy-contracts";
import { readError } from "./formatters";
import type { LocalAppInstallTask } from "../local-app-install-tasks";
import type { ConnectorSummary, LocalAppRuntimeStatus, ManagedToolStatus, MarketConnector, RuntimeSnapshot, StartConnectorAppInstallRequest, StartRegisteredServiceResult } from "./types";
import type { AppControllerState } from "./use-app-state";

interface LocalAppCatalogDependencies { applyRuntimeSnapshot: (snapshot: RuntimeSnapshot) => void; formatApplyMessage: (base: string, snapshot: RuntimeSnapshot) => string; handleCommandError: (error: unknown) => void; installableMarketConnectors: MarketConnector[]; refreshRegisteredServiceStatuses: () => Promise<LocalAppRuntimeStatus[] | null>; }

export function createLocalAppCatalogActions(state: AppControllerState, dependencies: LocalAppCatalogDependencies) {
  const { applyRuntimeSnapshot, formatApplyMessage, handleCommandError, installableMarketConnectors, refreshRegisteredServiceStatuses } = dependencies;
  const { authorizationStateRef, connectorAppsRef, customInstallConfirmed, installSourceMode, localAppInstallTasksRef, localAppUpdateRefreshRef, registeredInstallAppId, registeredInstallVersion, selectedMarketAppId, setActivePage, setBaijimuCli, setConnectorApps, setCustomInstallConfirmed, setError, setInstallBusy, setInstallPanelOpen, setInstallSourceMode, setLocalAppInstallTasks, setMarketAppQuery, setMarketConnectors, setMarketLoadError, setMarketLoading, setMessage, setPendingUpgradeAppId, setRuntimeConflict, setSelectedLocalAppId, setSelectedMarketAppId, setServiceStartBusy } = state;
  async function openLocalAppDeepLinkIntent(intent: LocalAppInstallDeepLinkIntent) {
    setActivePage("apps");
    setMessage("");
    setError("");

    const installedApps = await refreshConnectorApps();
    const installedApp = installedApps.find((app) => app.appId === intent.appId);
    if (installedApp) {
      setInstallPanelOpen(false);
      setSelectedLocalAppId(`connector:${installedApp.appId}`);
      setMessage(intent.shareId ? `已从分享入口打开 ${installedApp.name}` : `${installedApp.name} 已安装`);
      return;
    }

    let tasks = localAppInstallTasksRef.current;
    try {
      tasks = await invoke<LocalAppInstallTask[]>("list_connector_app_install_tasks");
      localAppInstallTasksRef.current = tasks;
      setLocalAppInstallTasks(
        [...tasks].sort((left, right) => left.createdAtEpochMs - right.createdAtEpochMs)
      );
    } catch (err) {
      clientWarn("读取本地应用安装进度失败", err);
    }
    const activeTask = [...tasks]
      .sort((left, right) => right.updatedAtEpochMs - left.updatedAtEpochMs)
      .find(
        (task) =>
          task.appId === intent.appId &&
          task.phase !== "succeeded" &&
          task.phase !== "failed"
      );
    if (activeTask) {
      setInstallPanelOpen(false);
      setSelectedLocalAppId(`install-task:${activeTask.taskId}`);
      setMessage("应用正在安装，已打开安装进度");
      return;
    }

    setSelectedLocalAppId(null);
    setInstallSourceMode("market");
    setMarketAppQuery("");
    setCustomInstallConfirmed(false);
    setInstallPanelOpen(true);
    const marketApps = await refreshMarketConnectorApps();
    const matches = marketApps.filter((app) => app.appId === intent.appId);
    if (matches.length > 1) {
      setError("该应用 ID 存在多个发布来源，请在市场中明确选择来源");
      return;
    }
    const marketApp = matches[0];
    if (marketApp) {
      setSelectedMarketAppId(marketSelectionKey(marketApp));
      setMessage(intent.shareId ? `已从分享入口打开 ${marketApp.name} 安装` : `已打开 ${marketApp.name} 安装`);
    } else {
      setError(`应用 ${intent.appId} 未公开上架或已撤销`);
    }
  }

  function openBaijimuDeepLinkIntent(intent: BaijimuDeepLinkIntent) {
    if (intent.kind === "local_app_install") {
      if (authorizationStateRef.current !== "authorized") {
        setActivePage("apps");
        setInstallPanelOpen(false);
        setSelectedLocalAppId(null);
        setPendingUpgradeAppId(null);
        clientWarn("设备未授权，忽略本地应用安装链接");
        return;
      }
      void openLocalAppDeepLinkIntent(intent);
      return;
    }

    clientInfo("收到通用客户端唤起请求");
  }

  async function refreshConnectorApps(): Promise<ConnectorSummary[]> {
    try {
      const apps = await invoke<ConnectorSummary[]>("list_connector_apps");
      const normalizedApps = apps.map((app) => ({
        ...app,
        configSchema: app.configSchema ?? null,
        database: app.database ?? null,
        methods: app.methods ?? legacyMethodContracts(app.methodNames ?? []),
        events: app.events ?? legacyEventContracts(app.eventNames ?? [])
      }));
      connectorAppsRef.current = normalizedApps;
      setConnectorApps(normalizedApps);
      return normalizedApps;
    } catch (err) {
      clientWarn("读取本地应用列表失败", err);
      return connectorAppsRef.current;
    }
  }

  async function refreshBaijimuCli() {
    try {
      setBaijimuCli(await invoke<ManagedToolStatus>("baijimu_cli_status"));
    } catch (err) {
      clientWarn("读取 Baijimu CLI 托管状态失败", err);
      setBaijimuCli(null);
    }
  }

  async function refreshMarketConnectorApps(): Promise<MarketConnector[]> {
    try {
      setMarketLoading(true);
      setMarketLoadError("");
      const apps = await invoke<MarketConnector[]>("list_market_connector_apps");
      const normalizedApps = apps.map((app) => ({
        ...app,
        releaseNotes: app.releaseNotes ?? [],
        configurationDeclaration: app.configurationDeclaration ?? "undeclared",
        interfaceDeclaration: app.interfaceDeclaration ?? "undeclared",
        databaseDeclaration: app.databaseDeclaration ?? "undeclared",
        configSchema: app.configSchema ?? null,
        database: app.database ?? null,
        methods: app.methods ?? legacyMethodContracts(app.methodNames ?? []),
        events: app.events ?? legacyEventContracts(app.eventNames ?? []),
        methodNames: app.methodNames ?? [],
        eventNames: app.eventNames ?? [],
        permissions: app.permissions ?? []
      }));
      setMarketConnectors(normalizedApps);
      return normalizedApps;
    } catch (err) {
      clientWarn("读取本地应用市场失败", err);
      setMarketConnectors([]);
      setMarketLoadError(readError(err));
      return [];
    } finally {
      setMarketLoading(false);
    }
  }

  function refreshLocalAppUpdateData(): Promise<void> {
    if (localAppUpdateRefreshRef.current) {
      return localAppUpdateRefreshRef.current;
    }
    const refresh = Promise.all([refreshMarketConnectorApps(), refreshBaijimuCli()])
      .then(() => undefined)
      .finally(() => {
        if (localAppUpdateRefreshRef.current === refresh) {
          localAppUpdateRefreshRef.current = null;
        }
      });
    localAppUpdateRefreshRef.current = refresh;
    return refresh;
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
        applyRuntimeSnapshot(snapshot);
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

  function applyLocalAppInstallTask(task: LocalAppInstallTask) {
    setLocalAppInstallTasks((current) => {
      const next = current.filter((candidate) => candidate.taskId !== task.taskId);
      next.push(task);
      return next.sort((left, right) => left.createdAtEpochMs - right.createdAtEpochMs);
    });
  }

  async function startLocalAppInstallTask(
    request: StartConnectorAppInstallRequest
  ): Promise<LocalAppInstallTask> {
    const onEvent = new Channel<LocalAppInstallTask>();
    onEvent.onmessage = applyLocalAppInstallTask;
    const task = await invoke<LocalAppInstallTask>("start_connector_app_install", {
      request,
      onEvent
    });
    applyLocalAppInstallTask(task);
    return task;
  }

  async function installLocalApp() {
    const selectedMarket = installableMarketConnectors.find((app) => marketSelectionKey(app) === selectedMarketAppId);
    if (installSourceMode === "market" && selectedMarket?.compatible === false) {
      setError(
        selectedMarket.compatibilityMessage ||
          "当前百积木客户端不支持该应用版本，请先升级客户端"
      );
      return;
    }
    const appId =
      installSourceMode === "market" ? selectedMarket?.appId ?? "" : registeredInstallAppId;
    const version =
      installSourceMode === "market" ? selectedMarket?.version ?? "" : registeredInstallVersion;
    if (!appId || !version) {
      setError(
        installSourceMode === "market"
          ? "请选择要安装的应用"
          : "请输入已在平台注册的 appId 和精确版本"
      );
      return;
    }
    if (installSourceMode === "custom" && !customInstallConfirmed) {
      setError("请先确认注册来源权限");
      return;
    }
    try {
      setInstallBusy(true);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      if (installSourceMode === "market" && selectedMarket?.applicationType === "managed_tool") {
        const status = await invoke<ManagedToolStatus>("install_baijimu_cli_update", { installSource: selectedMarket.installSource });
        setBaijimuCli(status);
        setInstallPanelOpen(false);
        setMessage(`${status.name} 已安装到 ${status.installedVersion}`);
        return;
      }
      const task = await startLocalAppInstallTask({
        installSource: installSourceMode === "market" ? selectedMarket?.installSource : null,
        operation: "install",
        replace: true,
        appId,
        name: installSourceMode === "market" ? selectedMarket?.name ?? null : null,
        version,
        acceptUnreviewed: installSourceMode === "custom" && customInstallConfirmed
      });
      setSelectedLocalAppId(`install-task:${task.taskId}`);
      setActivePage("apps");
      setInstallPanelOpen(false);
      setCustomInstallConfirmed(false);
      setMessage(`应用 ${task.name} 已开始后台安装，可在应用面板查看进度`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setInstallBusy(false);
    }
  }

  return { openLocalAppDeepLinkIntent, openBaijimuDeepLinkIntent, refreshConnectorApps, refreshBaijimuCli, refreshMarketConnectorApps, refreshLocalAppUpdateData, startRegisteredService, stopRegisteredService, applyLocalAppInstallTask, startLocalAppInstallTask, installLocalApp };
}
