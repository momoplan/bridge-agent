import { invoke } from "@tauri-apps/api/core";
import { flushSync } from "react-dom";
import { isConnectorUninstallStopError } from "../local-app-uninstall";
import { defaultCapabilityArgumentsText, fromUiConfig, toUiMethod } from "./config-conversion";
import { compareVersions, formatConnectorServiceFailures, parseJson, readError } from "./formatters";
import type { createLocalAppCatalogActions } from "./local-app-catalog-actions";
import type { createLocalAppSelectors } from "./local-app-selectors";
import type { CapabilityInvokeResult, ConfigDocument, ConnectorAppUpdateStatus, ConnectorStartResult, LocalAppItem, LocalAppRuntimeStatus, LocalAppUpdateStatus, ManagedToolStatus, MarketConnector, RuntimeSnapshot, UiServiceConfig } from "./types";
import type { AppControllerState } from "./use-app-state";

type CatalogDependencies = Pick<ReturnType<typeof createLocalAppCatalogActions>, "refreshConnectorApps" | "refreshLocalAppUpdateData" | "startLocalAppInstallTask" | "startRegisteredService" | "stopRegisteredService">;
type SelectorDependencies = Pick<ReturnType<typeof createLocalAppSelectors>, "clearLocalAppLifecycleOverride" | "hasLocalAppStartCommand" | "hasLocalAppStopCommand" | "setLocalAppLifecycleOverride">;
interface LocalAppLifecycleBaseDependencies { applyConfigDocument: (document: ConfigDocument) => void; applyRuntimeSnapshot: (snapshot: RuntimeSnapshot) => void; buildCapabilityTestKey: (serviceIndex: number, methodIndex: number) => string; formatApplyMessage: (base: string, snapshot: RuntimeSnapshot) => string; handleCommandError: (error: unknown) => void; refreshRegisteredServiceStatuses: () => Promise<LocalAppRuntimeStatus[] | null>; refreshRuntime: () => Promise<void>; }
type LocalAppLifecycleDependencies = CatalogDependencies & SelectorDependencies & LocalAppLifecycleBaseDependencies;

export function createLocalAppLifecycleActions(state: AppControllerState, dependencies: LocalAppLifecycleDependencies) {
  const { applyConfigDocument, applyRuntimeSnapshot, buildCapabilityTestKey, clearLocalAppLifecycleOverride, formatApplyMessage, handleCommandError, hasLocalAppStartCommand, hasLocalAppStopCommand, refreshConnectorApps, refreshLocalAppUpdateData, refreshRegisteredServiceStatuses, refreshRuntime, setLocalAppLifecycleOverride, startLocalAppInstallTask, startRegisteredService, stopRegisteredService } = dependencies;
  const { capabilityTestDrafts, config, marketConnectors, setBaijimuCli, setCapabilityTestBusy, setCapabilityTestResults, setConnectorBusy, setConnectorUninstalling, setConnectorUpdateBusy, setConnectorUpdateStatuses, setError, setManagedToolBusy, setMessage, setPendingUpgradeAppId, setRuntimeConflict, setSelectedLocalAppId } = state;
  function marketConnectorForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    if (app.kind !== "connector" || !app.connector || app.connector.reviewStatus !== "PUBLISHED") {
      return undefined;
    }
    return marketConnectors.find((marketApp) => marketApp.appId === app.connector?.appId);
  }

  function marketManagedToolForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    if (app.kind !== "managed_tool" || !app.managedTool) {
      return undefined;
    }
    return marketConnectors.find(
      (marketApp) =>
        marketApp.applicationType === "managed_tool" && marketApp.appId === app.managedTool?.id
    );
  }

  function marketAppForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    return app.kind === "managed_tool"
      ? marketManagedToolForLocalApp(app)
      : marketConnectorForLocalApp(app);
  }

  function localAppUpdateStatus(app: LocalAppItem): LocalAppUpdateStatus | undefined {
    const marketApp = marketAppForLocalApp(app);
    const currentVersion = app.managedTool?.installedVersion ?? app.connector?.version ?? null;
    if (!marketApp || !currentVersion) {
      return undefined;
    }
    return {
      appId: app.id,
      name: app.name,
      currentVersion,
      latestVersion: marketApp.version,
      updateAvailable: compareVersions(marketApp.version, currentVersion) > 0,
      source: marketApp.source
    };
  }

  async function upgradeLocalAppVersion(app: LocalAppItem) {
    if (app.kind === "managed_tool") {
      await upgradeManagedTool(app);
      return;
    }
    await upgradeLocalApp(app);
  }

  async function checkLocalAppVersion(app: LocalAppItem) {
    await refreshLocalAppUpdateData();
    if (app.connector) {
      await checkLocalAppUpdate(app);
      return;
    }
    setMessage(`${app.name} 的市场版本信息已刷新`);
  }

  async function upgradeManagedTool(app: LocalAppItem) {
    const marketApp = marketManagedToolForLocalApp(app);
    if (!app.managedTool || !marketApp) {
      setError(`工具 ${app.name} 没有关联的官方更新源`);
      return;
    }
    if (!marketApp.compatible) {
      setError(
        marketApp.compatibilityMessage ||
          `当前百积木客户端不支持 ${marketApp.name} ${marketApp.version}，请先升级客户端`
      );
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
      setPendingUpgradeAppId(null);
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
    if (app.connector?.reviewStatus === "PUBLISHED" && marketApp) {
      return "公开市场（已审核）";
    }
    return "平台注册版本（未公开审核）";
  }

  async function checkLocalAppUpdate(app: LocalAppItem, showLatestMessage = true) {
    const marketApp = marketConnectorForLocalApp(app);
    if (!app.connector || !marketApp) {
      setError(`应用 ${app.name} 没有关联的市场更新源`);
      return null;
    }
    if (!marketApp.compatible) {
      setError(
        marketApp.compatibilityMessage ||
          `当前百积木客户端不支持 ${marketApp.name} ${marketApp.version}，请先升级客户端`
      );
      return null;
    }
    try {
      setConnectorUpdateBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const status = await invoke<ConnectorAppUpdateStatus>("check_connector_app_update", {
        appId: app.connector.appId
      });
      setConnectorUpdateStatuses((current) => ({
        ...current,
        [app.connector!.appId]: status
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
    if (!marketApp.compatible) {
      setError(
        marketApp.compatibilityMessage ||
          `当前百积木客户端不支持 ${marketApp.name} ${marketApp.version}，请先升级客户端`
      );
      return;
    }
    try {
      setConnectorUpdateBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const task = await startLocalAppInstallTask({
        operation: "upgrade",
        replace: true,
        appId: app.connector.appId,
        name: marketApp.name,
        version: marketApp.version,
        acceptUnreviewed: false
      });
      setPendingUpgradeAppId(null);
      setSelectedLocalAppId(`connector:${app.connector.appId}`);
      setMessage(`应用 ${task.name} 已开始后台升级，可关闭详情并在应用卡片查看进度`);
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
    if (app.connector.reviewStatus === "PUBLISHED") {
      setError(`应用 ${app.name} 已公开上架，请通过市场检查更新`);
      return;
    }
    if (
      !window.confirm(
        `“${app.name}”来自已注册但未公开审核的来源。继续会向平台注册中心重新校验版本并覆盖当前安装内容，是否继续？`
      )
    ) {
      return;
    }
    try {
      setConnectorUpdateBusy(app.id);
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      const task = await startLocalAppInstallTask({
        operation: "sync",
        replace: true,
        appId: app.connector.appId,
        name: app.name,
        version: app.connector.version,
        acceptUnreviewed: true
      });
      setConnectorUpdateStatuses((current) => {
        const next = { ...current };
        delete next[app.connector!.appId];
        return next;
      });
      setSelectedLocalAppId(`connector:${app.connector.appId}`);
      setMessage(`应用 ${task.name} 已开始后台同步，可关闭详情并在应用卡片查看进度`);
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
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      if (app.kind === "connector" && app.connector) {
        const result = await invoke<ConnectorStartResult>("start_connector_app", {
          appId: app.connector.appId
        });
        const failed = result.lifecycle.exitCode !== 0 ? [result.lifecycle] : [];
        const statuses = await refreshRegisteredServiceStatuses();
        if (failed.length > 0) {
          setError(
            `应用 ${app.name} 启动失败：` +
              formatConnectorServiceFailures(failed)
          );
        } else {
          const runtimeStatus = statuses?.find(
            (status) => status.appId === app.connector!.appId
          );
          if (statuses && runtimeStatus?.status !== "healthy") {
            throw new Error(`应用 ${app.name} 启动后未进入健康运行状态`);
          }
          const snapshot = await invoke<RuntimeSnapshot>("apply_saved_config_to_runtime");
          applyRuntimeSnapshot(snapshot);
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
      setLocalAppLifecycleOverride(app.id, {
        state: "starting",
        detail: "正在执行应用启动命令"
      });
      const started = await startRegisteredService(startableService.name);
      setLocalAppLifecycleOverride(app.id, {
        state: started ? "ready" : "failed",
        detail: started ? "启动命令已执行" : "启动命令执行失败"
      });
    } catch (err) {
      if (app.kind !== "connector") {
        setLocalAppLifecycleOverride(app.id, {
          state: "failed",
          detail: readError(err)
        });
      }
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
      setMessage("");
      setError("");
      setRuntimeConflict(null);
      if (app.kind === "connector" && app.connector) {
        const result = await invoke<ConnectorStartResult>("stop_connector_app", {
          appId: app.connector.appId
        });
        const failed = result.lifecycle.exitCode !== 0 ? [result.lifecycle] : [];
        const statuses = await refreshRegisteredServiceStatuses();
        if (failed.length > 0) {
          clearLocalAppLifecycleOverride(app.id);
          setError(
            `应用 ${app.name} 停止失败：` +
              formatConnectorServiceFailures(failed)
          );
        } else {
          const runtimeStatus = statuses?.find(
            (status) => status.appId === app.connector!.appId
          );
          if (runtimeStatus?.status === "healthy") {
            throw new Error(`应用 ${app.name} 停止后仍处于健康运行状态`);
          }
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
      setLocalAppLifecycleOverride(app.id, {
        state: "stopping",
        detail: "正在执行应用停止命令"
      });
      const stopped = await stopRegisteredService(stoppableService.name);
      if (stopped) {
        setLocalAppLifecycleOverride(app.id, {
          state: "stopped",
          detail: "停止命令已执行"
        });
      }
    } catch (err) {
      clearLocalAppLifecycleOverride(app.id);
      await refreshRegisteredServiceStatuses();
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

  async function testLocalAppCapability(app: LocalAppItem, methodIndex: number) {
    const localApp =
      app.localAppIndex == null ? null : config?.local_apps[app.localAppIndex];
    const method = localApp?.methods[methodIndex];
    if (!config || !localApp || !method) {
      return;
    }
    const testKey = `local-app:${localApp.appId}:${methodIndex}`;
    const draft = capabilityTestDrafts[testKey] ?? defaultCapabilityArgumentsText(toUiMethod(method));
    try {
      setCapabilityTestBusy(testKey);
      setError("");
      const result = await invoke<CapabilityInvokeResult>("test_local_app_capability", {
        config: fromUiConfig(config),
        appId: localApp.appId,
        method: method.name,
        arguments: parseJson(draft)
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
        [testKey]: { status: "error", message: readError(err) }
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
      flushSync(() => {
        setConnectorBusy(app.id);
        setConnectorUninstalling(app.id);
        setMessage("");
        setError("");
        setRuntimeConflict(null);
      });
      let document: ConfigDocument;
      try {
        document = await invoke<ConfigDocument>("uninstall_connector_app", {
          appId: app.connector.appId,
          force: false
        });
      } catch (gracefulError) {
        if (!isConnectorUninstallStopError(gracefulError)) {
          throw gracefulError;
        }
        const detail = readError(gracefulError);
        const confirmed = window.confirm(
          `应用 ${app.name} 无法正常停止：\n\n${detail}\n\n是否强制终止该应用包内的进程并继续卸载？`
        );
        if (!confirmed) {
          throw gracefulError;
        }
        document = await invoke<ConfigDocument>("uninstall_connector_app", {
          appId: app.connector.appId,
          force: true
        });
      }
      applyConfigDocument(document);
      await refreshConnectorApps();
      await refreshRegisteredServiceStatuses();
      setSelectedLocalAppId(null);
      setMessage(`应用 ${app.name} 已卸载`);
    } catch (err) {
      handleCommandError(err);
    } finally {
      setConnectorUninstalling(null);
      setConnectorBusy(null);
    }
  }

  return { marketConnectorForLocalApp, marketManagedToolForLocalApp, marketAppForLocalApp, localAppUpdateStatus, upgradeLocalAppVersion, checkLocalAppVersion, upgradeManagedTool, rollbackManagedTool, connectorSyncSource, connectorSourceKind, checkLocalAppUpdate, upgradeLocalApp, syncLocalApp, startLocalApp, stopLocalApp, testCapability, testLocalAppCapability, uninstallLocalApp };
}
