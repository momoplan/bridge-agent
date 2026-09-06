import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { useEffect } from "react";
import { clientWarn } from "../client-logger";
import { parseBaijimuDeepLink } from "../deep-link";
import { resolveStartupUpdateGate } from "../startup-update-gate";
import type { createDesktopActions } from "./desktop-actions";
import type { createLocalAppCatalogActions } from "./local-app-catalog-actions";
import type { ConnectorLifecycleSnapshot, LocalAppRuntimeStatus, LocalAppsChangedEvent, RegisteredServiceStatus, RuntimeSnapshot, StartupHealthSnapshot } from "./types";
import type { LocalAppInstallTask } from "../local-app-install-tasks";
import type { AppControllerState } from "./use-app-state";

type ActionDependencies = ReturnType<typeof createDesktopActions> & ReturnType<typeof createLocalAppCatalogActions>;
interface EffectDependencies { applyRuntimeSnapshot: (snapshot: RuntimeSnapshot) => void; applyStartupHealthSnapshot: (snapshot: StartupHealthSnapshot) => void; refreshAll: () => Promise<void>; refreshDesktopPermissions: () => Promise<void>; refreshLocalAppCatalog: (revision: number) => Promise<void>; }

export function usePlatformEffects(context: AppControllerState & ActionDependencies & EffectDependencies) {
  const { activePage, applyRuntimeSnapshot, applyStartupHealthSnapshot, authorizationStateRef, checkAppUpdate, connectorApps, connectorAppsRef, localAppInstallTasks, localAppInstallTasksRef, localAppsChangeRevisionRef, openBaijimuDeepLinkIntent, previousActivePageRef, queuedDeepLinkIntentsRef, refreshAll, refreshDesktopPermissions, refreshLocalAppCatalog, refreshLocalAppUpdateData, setConnectorLifecycles, setLocalAppInstallTasks, setLocalAppRuntimeStatuses, setRegisteredServiceStatuses, setStartupUpdateGate, startupConfigGateReady } = context;
useEffect(() => {
    connectorAppsRef.current = connectorApps;
  }, [connectorApps]);

useEffect(() => {
    localAppInstallTasksRef.current = localAppInstallTasks;
  }, [localAppInstallTasks]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    let initialized = false;
    let unlisten: (() => void) | null = null;
    const queuedUrls: string[] = [];

    const dispatchUrl = (rawUrl: string) => {
      const intent = parseBaijimuDeepLink(rawUrl);
      if (!intent) {
        clientWarn("忽略不支持的百积木客户端链接", rawUrl);
        return;
      }
      if (authorizationStateRef.current == null) {
        queuedDeepLinkIntentsRef.current.push(intent);
        return;
      }
      openBaijimuDeepLinkIntent(intent);
    };

    async function initializeDeepLinks() {
      try {
        const dispose = await onOpenUrl((urls) => {
          if (!active) {
            return;
          }
          if (!initialized) {
            queuedUrls.push(...urls);
            return;
          }
          urls.forEach(dispatchUrl);
        });
        if (!active) {
          dispose();
          return;
        }
        unlisten = dispose;

        const startupUrls = await getCurrent();
        if (!active) {
          return;
        }
        initialized = true;
        Array.from(new Set([...(startupUrls ?? []), ...queuedUrls])).forEach(dispatchUrl);
      } catch (err) {
        clientWarn("初始化百积木客户端链接失败", err);
      }
    }

    void initializeDeepLinks();
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<RuntimeSnapshot>("runtime-snapshot-changed", (event) => {
      if (active) {
        applyRuntimeSnapshot(event.payload);
      }
    }).then((dispose) => {
      if (active) {
        unlisten = dispose;
      } else {
        dispose();
      }
    }).catch((err) => clientWarn("订阅 Agent 运行状态失败", err));
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    void invoke<LocalAppInstallTask[]>("list_connector_app_install_tasks")
      .then((tasks) => {
        if (active) {
          setLocalAppInstallTasks(tasks.sort((left, right) => left.createdAtEpochMs - right.createdAtEpochMs));
        }
      })
      .catch((err) => clientWarn("读取本地应用安装任务失败", err));
    return () => {
      active = false;
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<LocalAppsChangedEvent>("local-apps-changed", (event) => {
      if (!active || event.payload.revision <= localAppsChangeRevisionRef.current) {
        return;
      }
      localAppsChangeRevisionRef.current = event.payload.revision;
      void refreshLocalAppCatalog(event.payload.revision);
    }).then((dispose) => {
      if (active) {
        unlisten = dispose;
      } else {
        dispose();
      }
    }).catch((err) => clientWarn("订阅本地应用目录变更失败", err));
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    async function initializeStartupHealth() {
      try {
        const dispose = await listen<StartupHealthSnapshot>("startup-health-changed", (event) => {
          if (active) {
            applyStartupHealthSnapshot(event.payload);
          }
        });
        if (!active) {
          dispose();
          return;
        }
        unlisten = dispose;
        const snapshot = await invoke<StartupHealthSnapshot>("mark_frontend_ready");
        if (active) {
          applyStartupHealthSnapshot(snapshot);
        }
      } catch (err) {
        clientWarn("桌面基础壳启动握手失败", err);
      }
    }
    void initializeStartupHealth();
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<RegisteredServiceStatus[]>("registered-services-changed", (event) => {
      if (active) {
        setRegisteredServiceStatuses(event.payload);
      }
    }).then((dispose) => {
      if (active) {
        unlisten = dispose;
      } else {
        dispose();
      }
    }).catch((err) => clientWarn("订阅本地应用运行状态失败", err));
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<LocalAppRuntimeStatus[]>("local-app-runtime-changed", (event) => {
      if (!active) return;
      setLocalAppRuntimeStatuses(event.payload);
    }).then((dispose) => {
      if (active) unlisten = dispose;
      else dispose();
    }).catch((err) => clientWarn("订阅 Connector 运行状态失败", err));
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<ConnectorLifecycleSnapshot>("connector-lifecycle-changed", (event) => {
      if (!active) return;
      setConnectorLifecycles((current) => ({
        ...current,
        [event.payload.appId]: event.payload
      }));
    }).then((dispose) => {
      if (active) unlisten = dispose;
      else dispose();
    }).catch((err) => clientWarn("订阅 Connector 生命周期失败", err));
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startupConfigGateReady]);

useEffect(() => {
    if (startupConfigGateReady) {
      void refreshAll();
    }
  }, [startupConfigGateReady]);

useEffect(() => {
    if (!startupConfigGateReady) {
      return;
    }
    const previousPage = previousActivePageRef.current;
    previousActivePageRef.current = activePage;
    if (activePage === "apps" && previousPage !== "apps") {
      void refreshLocalAppUpdateData();
    }
  }, [activePage, startupConfigGateReady]);

useEffect(() => {
    let active = true;
    void checkAppUpdate().then((status) => {
      if (active) {
        setStartupUpdateGate(resolveStartupUpdateGate(status));
      }
    });
    return () => {
      active = false;
    };
  }, []);

useEffect(() => {
    if (startupConfigGateReady) {
      void refreshDesktopPermissions();
    }
  }, [startupConfigGateReady]);
}
