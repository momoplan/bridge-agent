import { formatLocalAppLifecycle } from "./formatters";
import type { LocalAppItem, LocalAppKind, LocalAppLifecycle, LocalAppLifecycleOverride, LocalAppLifecycleState, ManagedToolStatus, RegisteredServiceStatus, UiAgentConfig, UiServiceConfig } from "./types";
import type { AppControllerState } from "./use-app-state";

export function createLocalAppSelectors(state: AppControllerState) {
  const { config, connectorLifecycles, localAppLifecycleOverrides, localAppRuntimeStatuses, registeredServiceStatuses, setLocalAppLifecycleOverrides } = state;
  function hasLocalAppStartCommand(app: LocalAppItem) {
    if (!config) {
      return false;
    }
    if (app.localAppIndex != null) {
      return Boolean(config.local_apps[app.localAppIndex]?.startCommand);
    }
    return app.serviceIndexes.some((serviceIndex) => Boolean(config.services[serviceIndex]?.start_command));
  }

  function hasLocalAppStopCommand(app: LocalAppItem) {
    if (!config) {
      return false;
    }
    if (app.localAppIndex != null) {
      return Boolean(config.local_apps[app.localAppIndex]?.stopCommand);
    }
    return app.serviceIndexes.some((serviceIndex) => Boolean(config.services[serviceIndex]?.stop_command));
  }

  function setLocalAppLifecycleOverride(appId: string, override: LocalAppLifecycleOverride) {
    setLocalAppLifecycleOverrides((current) => ({
      ...current,
      [appId]: override
    }));
  }

  function clearLocalAppLifecycleOverride(appId: string) {
    setLocalAppLifecycleOverrides((current) => {
      const next = { ...current };
      delete next[appId];
      return next;
    });
  }

  function localAppLifecycle(app: LocalAppItem): LocalAppLifecycle {
    if (!config) {
      return formatLocalAppLifecycle("recovering", "等待配置加载");
    }

    if (app.installTask) {
      if (app.installTask.phase === "failed") {
        return formatLocalAppLifecycle("failed", app.installTask.error || app.installTask.message);
      }
      if (app.installTask.phase !== "succeeded") {
        return formatLocalAppLifecycle(
          app.installTask.operation === "install" ? "installing" : "upgrading",
          app.installTask.message
        );
      }
      if (!app.connector) {
        return formatLocalAppLifecycle("stopped", app.installTask.message);
      }
    }

    if (app.managedTool) {
      const managedToolLifecycle: Record<ManagedToolStatus["state"], LocalAppLifecycleState> = {
        ready: "ready",
        missing: "absent",
        broken: "failed"
      };
      return formatLocalAppLifecycle(managedToolLifecycle[app.managedTool.state], app.managedTool.detail);
    }

    if (app.localAppIndex != null) {
      const localApp = config.local_apps[app.localAppIndex];
      if (!localApp) {
        return formatLocalAppLifecycle("failed", "安装记录与本地应用配置不一致");
      }
      const lifecycle = connectorLifecycles[localApp.appId];
      if (lifecycle) {
        return formatLocalAppLifecycle(
          lifecycle.lifecycle,
          lifecycle.error || lifecycle.detail || undefined
        );
      }
      const status = localAppRuntimeStatuses.find(
        (candidate) => candidate.appId === localApp.appId
      );
      if (status?.status === "healthy") {
        return formatLocalAppLifecycle(
          "ready",
          status.detail || (status.healthCheckConfigured ? "healthCheck 已通过" : "宿主管理进程正在运行")
        );
      }
      if (status?.status === "unhealthy") {
        return formatLocalAppLifecycle(
          status.processRunning ? "degraded" : "stopped",
          status.detail || "运行状态检查未通过"
        );
      }
      return hasLocalAppStartCommand(app)
        ? formatLocalAppLifecycle("stopped", "已安装，等待启动")
        : formatLocalAppLifecycle("stopped", "已安装");
    }

    const override = localAppLifecycleOverrides[app.id];
    if (override) {
      return formatLocalAppLifecycle(override.state, override.detail);
    }

    const services = app.serviceIndexes
      .map((serviceIndex) => config.services[serviceIndex])
      .filter((service): service is UiServiceConfig => Boolean(service));
    if (services.length === 0) {
      return formatLocalAppLifecycle("stopped", "已安装，尚未关联本地服务");
    }

    const statuses = services
      .map((service) => registeredServiceStatuses.find((status) => status.service === service.name))
      .filter((status): status is RegisteredServiceStatus => Boolean(status));
    if (statuses.some((status) => status.status === "healthy")) {
      return formatLocalAppLifecycle("ready", "healthCheck 已通过");
    }
    if (statuses.some((status) => status.status === "unhealthy")) {
      return formatLocalAppLifecycle("degraded", "healthCheck 未通过");
    }
    if (statuses.some((status) => status.status === "unknown")) {
      return formatLocalAppLifecycle("recovering", "等待运行状态检查");
    }
    if (hasLocalAppStartCommand(app)) {
      return formatLocalAppLifecycle("stopped", "已安装，等待手动启动");
    }
    return formatLocalAppLifecycle("stopped", "已安装");
  }

  function isLocalAppRunning(app: LocalAppItem) {
    return localAppLifecycle(app).state === "ready";
  }

  function countLocalAppCapabilities(app: LocalAppItem, agentConfig: UiAgentConfig) {
    if (app.localAppIndex != null) {
      const localApp = agentConfig.local_apps[app.localAppIndex];
      return localApp ? localApp.methods.length + localApp.events.length : 0;
    }
    return app.serviceIndexes.reduce((count, serviceIndex) => {
      const service = agentConfig.services[serviceIndex];
      return service ? count + service.methods.length : count;
    }, 0);
  }

  function formatLocalAppKind(kind: LocalAppKind) {
    const labels: Record<LocalAppKind, string> = {
      connector: "已安装应用",
      managed_tool: "官方工具",
      built_in: "内置应用",
      custom: "自定义应用"
    };
    return labels[kind];
  }


  return { hasLocalAppStartCommand, hasLocalAppStopCommand, setLocalAppLifecycleOverride, clearLocalAppLifecycleOverride, localAppLifecycle, isLocalAppRunning, countLocalAppCapabilities, formatLocalAppKind };
}
