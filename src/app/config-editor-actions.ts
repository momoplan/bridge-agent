import { DEFAULT_SAFE_COMMANDS, FULL_ACCESS_COMMAND, FULL_ACCESS_ROOT_DIR } from "./constants";
import { createServiceHealthCheck, createServiceStartCommand, createServiceStopCommand, createShellMethod, serviceCapabilitiesJson, toUiMethod } from "./config-conversion";
import { parseJson, readError } from "./formatters";
import type { RelayConfig, RuntimeConfig, ServiceCapabilitiesDocument, UiAgentConfig, UiMethodConfig, UiServiceConfig, UiServiceHealthCheck, UiServiceStartCommand } from "./types";
import type { AppControllerState } from "./use-app-state";

export function createConfigEditorActions(state: AppControllerState) {
  const { config, serviceJsonDrafts, setConfig, setExpandedServiceIndex, setServiceJsonDrafts, setServiceJsonErrors } = state;
  function updateRelay<K extends keyof RelayConfig>(key: K, value: RelayConfig[K]) {
    setConfig((current) =>
      current
        ? {
            ...current,
            relay: {
              ...current.relay,
              [key]: value
            }
          }
        : current
    );
  }

  function updatePlatform<K extends "base_url" | "workspace_id">(
    key: K,
    value: UiAgentConfig["platform"][K]
  ) {
    setConfig((current) =>
      current
        ? {
            ...current,
            platform: {
              ...current.platform,
              [key]: value
            }
          }
        : current
    );
  }

  function updateUpload<K extends "prepare_url" | "inline_limit_bytes" | "timeout_secs">(
    key: K,
    value: UiAgentConfig["upload"][K]
  ) {
    setConfig((current) =>
      current
        ? {
            ...current,
            upload: {
              ...current.upload,
              [key]: value
            }
          }
        : current
    );
  }

  function updateDevice<K extends "name" | "description" | "tags_text">(
    key: K,
    value: UiAgentConfig["device"][K]
  ) {
    setConfig((current) =>
      current
        ? {
            ...current,
            device: {
              ...current.device,
              [key]: value
            }
          }
        : current
    );
  }

  function updateRuntime<K extends keyof RuntimeConfig>(key: K, value: RuntimeConfig[K]) {
    setConfig((current) =>
      current
        ? {
            ...current,
            runtime: {
              ...current.runtime,
              [key]: value
            }
          }
        : current
    );
  }

  function updateService(
    serviceIndex: number,
    updater: (service: UiServiceConfig) => UiServiceConfig
  ) {
    setConfig((current) => {
      if (!current) {
        return current;
      }
      const services = current.services.map((service, index) =>
        index === serviceIndex ? updater(service) : service
      );
      return { ...current, services };
    });
  }

  function updateServiceHealthCheck(
    serviceIndex: number,
    updater: (healthCheck: UiServiceHealthCheck) => UiServiceHealthCheck
  ) {
    updateService(serviceIndex, (service) => {
      const current = service.health_check ?? createServiceHealthCheck();
      return {
        ...service,
        health_check: updater(current)
      };
    });
  }

  function updateServiceStartCommand(
    serviceIndex: number,
    updater: (startCommand: UiServiceStartCommand) => UiServiceStartCommand
  ) {
    updateService(serviceIndex, (service) => {
      const current = service.start_command ?? createServiceStartCommand();
      return {
        ...service,
        start_command: updater(current)
      };
    });
  }

  function updateServiceStopCommand(
    serviceIndex: number,
    updater: (stopCommand: UiServiceStartCommand) => UiServiceStartCommand
  ) {
    updateService(serviceIndex, (service) => {
      const current = service.stop_command ?? createServiceStopCommand();
      return {
        ...service,
        stop_command: updater(current)
      };
    });
  }

  function updateServiceJsonDraft(serviceIndex: number, value: string) {
    setServiceJsonDrafts((current) => ({
      ...current,
      [serviceIndex]: value
    }));
    setServiceJsonErrors((current) => {
      const next = { ...current };
      delete next[serviceIndex];
      return next;
    });
  }

  function resetServiceJsonDraft(serviceIndex: number) {
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
  }

  function applyServiceJsonDraft(serviceIndex: number) {
    const service = config?.services[serviceIndex];
    if (!service) {
      return;
    }
    try {
      const draft = serviceJsonDrafts[serviceIndex] ?? serviceCapabilitiesJson(service);
      const parsed = parseJson(draft) as Partial<ServiceCapabilitiesDocument>;
      if (!Array.isArray(parsed.methods)) {
        throw new Error("能力定义 JSON 必须包含 methods 数组");
      }
      const methods = parsed.methods.map(toUiMethod);
      updateService(serviceIndex, (current) => ({
        ...current,
        methods
      }));
      resetServiceJsonDraft(serviceIndex);
    } catch (err) {
      setServiceJsonErrors((current) => ({
        ...current,
        [serviceIndex]: readError(err)
      }));
    }
  }

  function addService() {
    const nextIndex = config?.services.length ?? 0;
    setExpandedServiceIndex(nextIndex);
    setConfig((current) =>
      current
        ? {
            ...current,
            services: [
              ...current.services,
              {
                name: "new-service",
                description: "Describe this business service.",
                enabled: true,
                health_check: null,
                start_command: null,
                stop_command: null,
                methods: [createShellMethod()]
              }
            ]
          }
        : current
    );
  }

  function removeService(serviceIndex: number) {
    setExpandedServiceIndex((current) => {
      if (current == null) {
        return current;
      }
      if (current === serviceIndex) {
        return null;
      }
      if (current > serviceIndex) {
        return current - 1;
      }
      return current;
    });
    setConfig((current) =>
      current
        ? {
            ...current,
            services: current.services.filter((_, index) => index !== serviceIndex)
          }
        : current
    );
  }

  function removeMethod(serviceIndex: number, methodIndex: number) {
    updateService(serviceIndex, (service) => ({
      ...service,
      methods: service.methods.filter((_, index) => index !== methodIndex)
    }));
  }

  function updateMethod(
    serviceIndex: number,
    methodIndex: number,
    updater: (method: UiMethodConfig) => UiMethodConfig
  ) {
    updateService(serviceIndex, (service) => ({
      ...service,
      methods: service.methods.map((method, index) =>
        index === methodIndex ? updater(method) : method
      )
    }));
  }

  function grantFullShellAccess(serviceIndex: number, methodIndex: number) {
    updateMethod(serviceIndex, methodIndex, (current) => {
      if (current.binding.type !== "shell_command") {
        return current;
      }
      return {
        ...current,
        binding: {
          ...current.binding,
          root_dir: FULL_ACCESS_ROOT_DIR,
          allow_commands_text: FULL_ACCESS_COMMAND
        }
      };
    });
  }

  function restoreSafeShellAccess(serviceIndex: number, methodIndex: number) {
    updateMethod(serviceIndex, methodIndex, (current) => {
      if (current.binding.type !== "shell_command") {
        return current;
      }
      return {
        ...current,
        binding: {
          ...current.binding,
          root_dir: ".",
          allow_commands_text: DEFAULT_SAFE_COMMANDS
        }
      };
    });
  }

  return { updateRelay, updatePlatform, updateUpload, updateDevice, updateRuntime, updateService, updateServiceHealthCheck, updateServiceStartCommand, updateServiceStopCommand, updateServiceJsonDraft, resetServiceJsonDraft, applyServiceJsonDraft, addService, removeService, removeMethod, updateMethod, grantFullShellAccess, restoreSafeShellAccess };
}
