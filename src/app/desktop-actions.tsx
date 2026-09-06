import { invoke } from "@tauri-apps/api/core";
import { clientWarn } from "../client-logger";
import { resolveStartupUpdateGate } from "../startup-update-gate";
import { buildConsoleUrl, fromUiConfig, fromUiService } from "./config-conversion";
import { formatAppUpdateProgressDetail, formatStartAgentMessage, needsBrowserAuthorization, readError, readRuntimeConflict, serviceSignature } from "./formatters";
import type { AppUpdateInstallResult, AppUpdateStatus, AppVersionInfo, BrowserAuthStartResponse, ConfigDocument, ConfigRecoveryDocument, DesktopPermissionStatus, LocalAppRuntimeStatus, MarketConnector, RuntimeSnapshot } from "./types";
import type { AppControllerState } from "./use-app-state";

interface DesktopActionDependencies { appUpdateProgressPercent: number | null; applyConfigDocument: (document: ConfigDocument) => void; applyDeletedServiceDocument: (document: ConfigDocument, serviceIndex: number) => void; applyRuntimeSnapshot: (snapshot: RuntimeSnapshot) => void; applySavedServiceDocument: (document: ConfigDocument, serviceIndex: number) => void; formatApplyMessage: (base: string, snapshot: RuntimeSnapshot) => string; handleCommandError: (error: unknown) => void; refreshRegisteredServiceStatuses: () => Promise<LocalAppRuntimeStatus[] | null>; refreshRuntime: () => Promise<void>; }

export function createDesktopActions(state: AppControllerState, dependencies: DesktopActionDependencies) {
  const { appUpdateProgressPercent, applyConfigDocument, applyDeletedServiceDocument, applyRuntimeSnapshot, applySavedServiceDocument, formatApplyMessage, handleCommandError, refreshRegisteredServiceStatuses, refreshRuntime } = dependencies;
  const { appUpdate, appUpdateProgress, config, logsClearedThroughRef, runtime, runtimeConflict, setActivePage, setAppUpdate, setAppUpdateCheckState, setAppUpdateError, setAppUpdateProgress, setAppVersion, setBrowserAuth, setBusy, setDesktopPermissionBusy, setDesktopPermissions, setError, setExpandedServiceIndex, setLogs, setMessage, setRuntimeConflict, setSavedServiceSignatures, setServiceNotices, setStartupRecoveryBusy, setStartupUpdateGate, setUpdateBusy, updateBusy } = state;
  async function checkAppUpdate(showLatestMessage = false): Promise<AppUpdateStatus | null> {
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
              ? `发现新版本 ${status.latestVersion}，可以通过官方签名更新器安装并重启。`
              : `发现新版本 ${status.latestVersion}，但当前平台需要跳转发布页手工下载。`
            : `当前已经是最新版本 ${status.currentVersion}`
        );
      }
      return status;
    } catch (err) {
      const message = readError(err);
      setAppUpdateCheckState("error");
      setAppUpdateError(message);
      if (showLatestMessage) {
        setError(message);
      } else {
        clientWarn("自动检查更新失败", err);
      }
      return null;
    }
  }

  async function checkStartupAppUpdate(showLatestMessage = false): Promise<AppUpdateStatus | null> {
    const status = await checkAppUpdate(showLatestMessage);
    setStartupUpdateGate(resolveStartupUpdateGate(status));
    return status;
  }

  async function loadAppVersion() {
    try {
      setAppVersion(await invoke<AppVersionInfo>("app_version"));
    } catch (err) {
      clientWarn("读取本地应用版本失败", err);
    }
  }

  async function openAppUninstaller() {
    try {
      setError("");
      await invoke("open_app_uninstaller");
      setMessage("卸载向导已打开，请在向导中选择是否保留本机数据");
    } catch (err) {
      handleCommandError(err);
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

  async function installAppUpdate(update = appUpdate) {
    try {
      setUpdateBusy(true);
      setAppUpdateProgress({
        phase: "checking",
        message: "正在获取最新版本信息",
        version: update?.latestVersion ?? null,
        assetName: update?.assetName ?? null,
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
        `签名更新 ${result.version} 已安装，应用即将重启。`
      );
    } catch (err) {
      setAppUpdateProgress(null);
      handleCommandError(err);
    } finally {
      setUpdateBusy(false);
    }
  }

  async function upgradeClientForMarketApp(app: MarketConnector) {
    let update = appUpdate;
    if (!update?.updateAvailable) {
      update = await checkAppUpdate(false);
    }
    if (!update?.updateAvailable) {
      setError(
        `应用 ${app.name} 需要百积木客户端 ${app.minimumHostVersion ?? "更高版本"}，但官方更新服务暂未提供可用的新版本。`
      );
      return;
    }
    if (update.autoDownloadAvailable) {
      await installAppUpdate(update);
      return;
    }
    if (update.releaseUrl) {
      await openAppUpdateReleasePage(update);
      return;
    }
    setError(`已发现客户端 ${update.latestVersion ?? "新版本"}，但当前平台没有可用的签名安装包或下载地址。`);
  }

  async function restartInNormalMode() {
    try {
      setStartupRecoveryBusy(true);
      setError("");
      await invoke("restart_in_normal_mode");
    } catch (err) {
      handleCommandError(err);
      setStartupRecoveryBusy(false);
    }
  }

  async function openStartupLog() {
    try {
      await invoke("open_startup_log");
    } catch (err) {
      handleCommandError(err);
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
    if (needsBrowserAuthorization(config, runtime)) {
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
      applyRuntimeSnapshot(snapshot);
      setMessage(formatStartAgentMessage(snapshot));
      await refreshRuntime();
    } catch (err) {
      const conflict = readRuntimeConflict(err);
      if (conflict) {
        setRuntimeConflict(conflict);
        setActivePage("apps");
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
      applyRuntimeSnapshot(snapshot);
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
      applyRuntimeSnapshot(snapshot);
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
      const clearedThrough = await invoke<number>("clear_logs");
      logsClearedThroughRef.current = Math.max(
        logsClearedThroughRef.current,
        clearedThrough
      );
      setLogs((current) =>
        current.filter((entry) => entry.sequence > logsClearedThroughRef.current)
      );
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

  async function openAppUpdateReleasePage(update: AppUpdateStatus) {
    if (!update.releaseUrl) {
      setError("更新服务未提供下载页地址。");
      return;
    }
    await openExternalUrl(update.releaseUrl);
  }

  async function openConsole() {
    if (!config) {
      return;
    }
    if (needsBrowserAuthorization(config, runtime)) {
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

  async function openDesktopPermissionSettings(
    permission: "accessibility" | "screen_recording" | "full_disk_access"
  ) {
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

  return { checkAppUpdate, checkStartupAppUpdate, loadAppVersion, openAppUninstaller, renderAppUpdateProgress, installAppUpdate, upgradeClientForMarketApp, restartInNormalMode, openStartupLog, saveConfig, saveService, deleteSavedService, startAgent, stopConflictingRuntimeAndStart, stopAgent, resetExampleConfig, recoverInvalidConfig, clearLogs, openExternalUrl, openAppUpdateReleasePage, openConsole, openExternalUrlInEdge, copyText, copyTextWithSelection, requestDesktopPermission, openDesktopPermissionSettings, beginBrowserAuth };
}
