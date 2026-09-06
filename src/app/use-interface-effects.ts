import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { clientWarn } from "../client-logger";
import { defaultLocalAppDetailTab } from "./embedded-local-app";
import { normalizePlatformBaseUrl } from "./config-conversion";
import { DEFAULT_PLATFORM_BASE_URL } from "./constants";
import { mergeRecentLogs, updateRuntimeLogStreaming } from "./runtime-log";
import { reconcileLocalAppInstallSelection } from "../local-app-install-tasks";
import { deriveDeviceAuthorizationState } from "../device-authorization-state";
import type { createDesktopActions } from "./desktop-actions";
import type { createLocalAppCatalogActions } from "./local-app-catalog-actions";
import type { AppPage, AppUpdateProgress, LocalAppItem, LogEntry, MarketConnector } from "./types";
import type { AppControllerState } from "./use-app-state";

type ActionDependencies = ReturnType<typeof createDesktopActions> & ReturnType<typeof createLocalAppCatalogActions>;
interface EffectDependencies { authorizationState: ReturnType<typeof deriveDeviceAuthorizationState>; localApps: LocalAppItem[]; needsAuthorization: boolean; refreshDesktopPermissions: () => Promise<void>; selectedLocalApp: LocalAppItem | null | undefined; visibleMarketConnectors: MarketConnector[]; }

export function useInterfaceEffects(context: AppControllerState & ActionDependencies & EffectDependencies) {
  const { activeDetailPanel, activePage, authorizationState, authorizationStateRef, config, installPanelOpen, localAppInstallTasks, localApps, logsClearedThroughRef, mainWindowVisible, message, needsAuthorization, openBaijimuDeepLinkIntent, pendingUpgradeAppId, queuedDeepLinkIntentsRef, refreshDesktopPermissions, refreshLocalAppUpdateData, selectedLocalApp, selectedLocalAppId, setActiveLocalAppDetailTab, setActivePage, setAppUpdateProgress, setExpandedServiceIndex, setInstallPanelOpen, setLogs, setMainWindowVisible, setMessage, setPendingUpgradeAppId, setSelectedLocalAppId, setSelectedMarketAppId, setShowAdvancedSettings, startupConfigGateReady, visibleMarketConnectors } = context;
useEffect(() => {
    const handleKeyboardShortcut = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey) {
        return;
      }
      const tagName = (event.target as HTMLElement | null)?.tagName;
      const editing = tagName === "INPUT" || tagName === "TEXTAREA" || tagName === "SELECT";
      if (editing && event.key !== ",") {
        return;
      }
      const pageByKey: Partial<Record<string, AppPage>> = {
        "1": "apps",
        "2": "diagnostics",
        "3": "settings",
        ",": "settings"
      };
      const page = pageByKey[event.key];
      if (page) {
        event.preventDefault();
        setActivePage(page);
        if (page === "apps") {
          setSelectedLocalAppId(null);
        }
      }
    };
    window.addEventListener("keydown", handleKeyboardShortcut);
    return () => window.removeEventListener("keydown", handleKeyboardShortcut);
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
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<boolean>("main-window-visibility-changed", (event) => {
      if (active) {
        setMainWindowVisible(event.payload);
      }
    }).then((dispose) => {
      if (active) {
        unlisten = dispose;
      } else {
        dispose();
      }
    }).catch((err) => clientWarn("订阅主窗口可见状态失败", err));
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    const handleWindowFocus = () => {
      void refreshDesktopPermissions();
      if (activePage === "apps") {
        void refreshLocalAppUpdateData();
      }
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshDesktopPermissions();
        if (activePage === "apps") {
          void refreshLocalAppUpdateData();
        }
      }
    };

    window.addEventListener("focus", handleWindowFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("focus", handleWindowFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [activePage, startupConfigGateReady]);

useEffect(() => {
    if (
      activePage !== "diagnostics" ||
      activeDetailPanel !== "logs" ||
      !mainWindowVisible
    ) {
      return;
    }
    setLogs([]);
    let active = true;
    let unlistenAppended: (() => void) | null = null;
    let unlistenSnapshot: (() => void) | null = null;
    async function initializeLogs() {
      try {
        const disposeAppended = await listen<LogEntry>("runtime-log-appended", (event) => {
          if (active && event.payload.sequence > logsClearedThroughRef.current) {
            setLogs((current) => mergeRecentLogs(current, [event.payload]));
          }
        });
        if (!active) {
          disposeAppended();
          return;
        }
        unlistenAppended = disposeAppended;
        const disposeSnapshot = await listen<LogEntry[]>("runtime-logs-snapshot", (event) => {
          if (active) {
            setLogs(
              mergeRecentLogs(
                event.payload.filter(
                  (entry) => entry.sequence > logsClearedThroughRef.current
                )
              )
            );
          }
        });
        if (!active) {
          disposeSnapshot();
          return;
        }
        unlistenSnapshot = disposeSnapshot;
        await updateRuntimeLogStreaming(true);
        if (!active) {
          return;
        }
        const history = await invoke<LogEntry[]>("list_logs", { limit: 200 });
        if (active) {
          const visibleHistory = history.filter(
            (entry) => entry.sequence > logsClearedThroughRef.current
          );
          setLogs((current) => mergeRecentLogs(visibleHistory, current));
        }
      } catch (err) {
        clientWarn("读取或订阅 Agent 日志失败", err);
      }
    }
    void initializeLogs();
    return () => {
      active = false;
      unlistenAppended?.();
      unlistenSnapshot?.();
      void updateRuntimeLogStreaming(false).catch((err) =>
        clientWarn("停止 Agent 日志订阅失败", err)
      );
    };
  }, [activePage, activeDetailPanel, mainWindowVisible]);

useEffect(() => {
    if (!config) {
      return;
    }
    if (normalizePlatformBaseUrl(config.platform.base_url) !== DEFAULT_PLATFORM_BASE_URL) {
      setShowAdvancedSettings(true);
    }
  }, [config]);

useEffect(() => {
    setSelectedMarketAppId((current) => {
      if (current && visibleMarketConnectors.some((app) => app.appId === current)) {
        return current;
      }
      return visibleMarketConnectors[0]?.appId ?? "";
    });
  }, [visibleMarketConnectors]);

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

useEffect(() => {
    if (!config) {
      return;
    }
    authorizationStateRef.current = authorizationState;
    const queuedIntents = queuedDeepLinkIntentsRef.current.splice(0);
    queuedIntents.forEach(openBaijimuDeepLinkIntent);
  }, [authorizationState, config]);

useEffect(() => {
    if (!needsAuthorization) {
      return;
    }
    setActivePage((current) => current === "diagnostics" ? current : "apps");
    setInstallPanelOpen(false);
    setSelectedLocalAppId(null);
    setPendingUpgradeAppId(null);
  }, [installPanelOpen, needsAuthorization, pendingUpgradeAppId, selectedLocalAppId]);

useEffect(() => {
    setSelectedLocalAppId((current) =>
      reconcileLocalAppInstallSelection(
        current,
        localApps.map((app) => app.id),
        localAppInstallTasks
      )
    );
    setPendingUpgradeAppId((current) =>
      current && localApps.some((app) => app.id === current) ? current : null
    );
  }, [localApps, localAppInstallTasks]);

useEffect(() => {
    if (selectedLocalApp?.serviceIndexes.length) {
      setExpandedServiceIndex(selectedLocalApp.serviceIndexes[0]);
    }
    setActiveLocalAppDetailTab(
      selectedLocalApp ? defaultLocalAppDetailTab(selectedLocalApp) : "overview"
    );
  }, [selectedLocalApp?.id]);
}
