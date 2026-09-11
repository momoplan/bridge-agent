import officialEnvironment from "../../config/official-environment.json";
import { DEFAULT_CONSOLE_BASE_URL, DEFAULT_PLATFORM_BASE_URL, DEFAULT_SAFE_COMMANDS, FULL_ACCESS_COMMAND, HTTP_SCHEMA, SHELL_SCHEMA } from "./constants";
import { headersToText, parseJson, prettyJson, sampleJsonValue, textToHeaders, toOptionalNumber, toOptionalText } from "./formatters";
import type { AgentConfig, EventConfig, MethodConfig, ServiceCapabilitiesDocument, ServiceConfig, ServiceHealthCheck, ServiceStartCommand, UiAgentConfig, UiEventConfig, UiMethodBinding, UiMethodConfig, UiServiceConfig, UiServiceHealthCheck, UiServiceStartCommand, UiShellBinding } from "./types";
export function toUiConfig(config: AgentConfig): UiAgentConfig {
  return {
    platform: {
      environment_key: config.platform.environment_key,
      base_url: normalizePlatformBaseUrl(config.platform.base_url),
      workspace_id:
        config.platform.workspace_id == null ? "" : String(config.platform.workspace_id)
    },
    upload: {
      prepare_url: config.upload.prepare_url ?? "",
      inline_limit_bytes: config.upload.inline_limit_bytes,
      timeout_secs: config.upload.timeout_secs
    },
    relay: { ...config.relay, token: "" },
    device: {
      name: config.device.name,
      description: config.device.description,
      tags_text: config.device.tags.join(", ")
    },
    runtime: config.runtime,
    services: config.services.map(toUiService),
    local_apps: config.local_apps ?? [],
    credential_status: {
      relay_token_configured:
        config.credential_status?.relay_token_configured === true || Boolean(config.relay.token.trim())
    }
  };
}

export function fromUiConfig(config: UiAgentConfig): AgentConfig {
  return {
    platform: {
      environment_key: config.platform.environment_key,
      base_url: normalizePlatformBaseUrl(config.platform.base_url),
      workspace_id: toOptionalNumber(config.platform.workspace_id)
    },
    upload: {
      prepare_url: emptyToNull(config.upload.prepare_url),
      inline_limit_bytes: config.upload.inline_limit_bytes,
      timeout_secs: config.upload.timeout_secs
    },
    relay: { ...config.relay, token: "" },
    device: {
      name: config.device.name.trim(),
      description: config.device.description.trim(),
      tags: splitCommaList(config.device.tags_text)
    },
    runtime: config.runtime,
    services: config.services.map(fromUiService),
    local_apps: config.local_apps
  };
}

export function toUiService(service: ServiceConfig): UiServiceConfig {
  return {
    name: service.name,
    description: service.description,
    enabled: service.enabled,
    health_check: service.health_check ? toUiServiceHealthCheck(service.health_check) : null,
    start_command: service.start_command ? toUiServiceStartCommand(service.start_command) : null,
    stop_command: service.stop_command ? toUiServiceStartCommand(service.stop_command) : null,
    methods: service.methods.map(toUiMethod)
  };
}

export function fromUiService(service: UiServiceConfig): ServiceConfig {
  return {
    name: service.name.trim(),
    description: service.description.trim(),
    enabled: service.enabled,
    health_check: service.health_check ? fromUiServiceHealthCheck(service.health_check) : null,
    start_command: service.start_command ? fromUiServiceStartCommand(service.start_command) : null,
    stop_command: service.stop_command ? fromUiServiceStartCommand(service.stop_command) : null,
    methods: service.methods.map(fromUiMethod)
  };
}

export function serviceCapabilitiesJson(service: UiServiceConfig): string {
  const document: ServiceCapabilitiesDocument = {
    methods: service.methods.map(fromUiMethod)
  };
  return prettyJson(document);
}

export function toUiMethod(method: MethodConfig): UiMethodConfig {
  return {
    name: method.name,
    description: method.description,
    enabled: method.enabled,
    input_schema_text: prettyJson(method.input_schema),
    binding:
      method.binding.type === "shell_command"
        ? {
            type: "shell_command",
            root_dir: method.binding.root_dir,
            allow_commands_text: method.binding.allow_commands.join(", "),
            default_timeout_secs: toOptionalText(method.binding.default_timeout_secs),
            max_timeout_secs: toOptionalText(method.binding.max_timeout_secs)
          }
        : method.binding.type === "http"
          ? {
              type: "http",
              url: method.binding.url,
              http_method: method.binding.http_method,
              headers_text: headersToText(method.binding.headers),
              timeout_secs: toOptionalText(method.binding.timeout_secs)
            }
          : {
              type: "computer_use",
              action: method.binding.action,
              display_id: toOptionalText(method.binding.display_id)
            }
  };
}

export function fromUiMethod(method: UiMethodConfig): MethodConfig {
  return {
    name: method.name.trim(),
    description: method.description.trim(),
    enabled: method.enabled,
    input_schema: parseJson(method.input_schema_text),
    binding:
      method.binding.type === "shell_command"
        ? {
            type: "shell_command",
            root_dir: method.binding.root_dir.trim(),
            allow_commands: splitCommaList(method.binding.allow_commands_text),
            default_timeout_secs: toOptionalNumber(method.binding.default_timeout_secs),
            max_timeout_secs: toOptionalNumber(method.binding.max_timeout_secs)
          }
        : method.binding.type === "http"
          ? {
              type: "http",
              url: method.binding.url.trim(),
              http_method: method.binding.http_method.trim().toUpperCase(),
              headers: textToHeaders(method.binding.headers_text),
              timeout_secs: toOptionalNumber(method.binding.timeout_secs)
            }
          : {
              type: "computer_use",
              action: method.binding.action,
              display_id: toOptionalNumber(method.binding.display_id)
            }
  };
}

export function toUiEvent(eventConfig: EventConfig): UiEventConfig {
  return {
    name: eventConfig.name,
    description: eventConfig.description,
    enabled: eventConfig.enabled,
    payload_schema_text: prettyJson(eventConfig.payload_schema)
  };
}

export function fromUiEvent(eventConfig: UiEventConfig): EventConfig {
  return {
    name: eventConfig.name.trim(),
    description: eventConfig.description.trim(),
    enabled: eventConfig.enabled,
    payload_schema: parseJson(eventConfig.payload_schema_text)
  };
}

export function normalizePlatformBaseUrl(value: string): string {
  const normalized = value.trim();
  if (!normalized) {
    return DEFAULT_PLATFORM_BASE_URL;
  }
  try {
    const url = new URL(normalized);
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
      return normalized;
    }
    const path = url.pathname.replace(/\/+$/, "");
    if (
      (url.origin === officialEnvironment.apiBaseUrl || officialEnvironment.legacyApiOrigins.includes(url.origin)) &&
      (path === "" || officialEnvironment.legacyApiPaths.includes(path))
    ) {
      return DEFAULT_PLATFORM_BASE_URL;
    }
    return normalized.replace(/\/+$/, "").replace(/\/lowcode3$/, "");
  } catch {
    return normalized;
  }
  return normalized.replace(/\/+$/, "");
}

export function buildConsoleUrl(config: UiAgentConfig): string {
  const platformBaseUrl = normalizePlatformBaseUrl(config.platform.base_url);
  if (platformBaseUrl === DEFAULT_PLATFORM_BASE_URL) {
    return DEFAULT_CONSOLE_BASE_URL;
  }
  try {
    return new URL("/manager", platformBaseUrl).toString();
  } catch {
    return DEFAULT_CONSOLE_BASE_URL;
  }
}

export function emptyToNull(value: string): string | null {
  const normalized = value.trim();
  return normalized ? normalized : null;
}

export function createShellMethod(): UiMethodConfig {
  return {
    name: "exec",
    description: "Run one allowlisted command with optional cwd and env.",
    enabled: true,
    input_schema_text: prettyJson(SHELL_SCHEMA),
    binding: {
      type: "shell_command",
      root_dir: ".",
      allow_commands_text: DEFAULT_SAFE_COMMANDS,
      default_timeout_secs: "",
      max_timeout_secs: ""
    }
  };
}

export function createHttpMethod(): UiMethodConfig {
  return {
    name: "invokeApi",
    description: "Forward invocation arguments to a local HTTP endpoint.",
    enabled: true,
    input_schema_text: prettyJson(HTTP_SCHEMA),
    binding: {
      type: "http",
      url: "http://127.0.0.1:8081/api/invoke",
      http_method: "POST",
      headers_text: "",
      timeout_secs: ""
    }
  };
}

export function createServiceHealthCheck(): UiServiceHealthCheck {
  return {
    type: "http",
    url: "http://127.0.0.1:8081/health",
    http_method: "GET",
    headers_text: "",
    timeout_secs: "3",
    expect_status: "200",
    body_contains: ""
  };
}

export function createServiceStartCommand(): UiServiceStartCommand {
  return {
    type: "shell_command",
    command_text: "npm\nrun\ndev",
    cwd: "",
    env_text: "",
    timeout_secs: "20"
  };
}

export function createServiceStopCommand(): UiServiceStartCommand {
  return {
    type: "shell_command",
    command_text: "",
    cwd: "",
    env_text: "",
    timeout_secs: "20"
  };
}

export function toUiServiceHealthCheck(healthCheck: ServiceHealthCheck): UiServiceHealthCheck {
  return {
    type: "http",
    url: healthCheck.url,
    http_method: healthCheck.http_method,
    headers_text: headersToText(healthCheck.headers),
    timeout_secs: toOptionalText(healthCheck.timeout_secs),
    expect_status: toOptionalText(healthCheck.expect_status),
    body_contains: healthCheck.body_contains ?? ""
  };
}

export function fromUiServiceHealthCheck(healthCheck: UiServiceHealthCheck): ServiceHealthCheck {
  return {
    type: "http",
    url: healthCheck.url.trim(),
    http_method: healthCheck.http_method.trim().toUpperCase(),
    headers: textToHeaders(healthCheck.headers_text),
    timeout_secs: toOptionalNumber(healthCheck.timeout_secs),
    expect_status: toOptionalNumber(healthCheck.expect_status),
    body_contains: emptyToNull(healthCheck.body_contains)
  };
}

export function toUiServiceStartCommand(startCommand: ServiceStartCommand): UiServiceStartCommand {
  return {
    type: "shell_command",
    command_text: startCommand.command.join("\n"),
    cwd: startCommand.cwd ?? "",
    env_text: headersToText(startCommand.env),
    timeout_secs: toOptionalText(startCommand.timeout_secs)
  };
}

export function fromUiServiceStartCommand(startCommand: UiServiceStartCommand): ServiceStartCommand {
  const command = splitLineList(startCommand.command_text);
  if (command.length === 0) {
    throw new Error("启动命令不能为空");
  }
  return {
    type: "shell_command",
    command,
    cwd: emptyToNull(startCommand.cwd),
    env: textToHeaders(startCommand.env_text),
    timeout_secs: toOptionalNumber(startCommand.timeout_secs)
  };
}

export function isComputerService(service: Pick<UiServiceConfig, "name">): boolean {
  return service.name.trim().toLowerCase() === "computer";
}

export function isShellService(service: Pick<UiServiceConfig, "name">): boolean {
  return service.name.trim().toLowerCase() === "shell";
}

export function isSystemService(service: Pick<UiServiceConfig, "name">): boolean {
  return isComputerService(service) || isShellService(service);
}

export function formatMethodTypeLabel(type: UiMethodBinding["type"]): string {
  if (type === "shell_command") {
    return "Shell";
  }
  if (type === "http") {
    return "HTTP";
  }
  return "Computer";
}

export function splitCommaList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function splitLineList(value: string): string[] {
  return value
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function reindexRecordAfterDelete(
  record: Record<number, string>,
  deletedIndex: number
): Record<number, string> {
  const next: Record<number, string> = {};
  for (const [rawIndex, value] of Object.entries(record)) {
    const index = Number(rawIndex);
    if (!Number.isInteger(index) || index === deletedIndex) {
      continue;
    }
    next[index > deletedIndex ? index - 1 : index] = value;
  }
  return next;
}

export function isFullShellAccess(binding: UiShellBinding): boolean {
  return splitCommaList(binding.allow_commands_text).includes(FULL_ACCESS_COMMAND);
}

export function describeMethodBinding(method: UiMethodConfig): string {
  if (method.binding.type === "shell_command") {
    return isFullShellAccess(method.binding) ? "Shell 调用，全部权限模式" : "Shell 调用，受限模式";
  }
  if (method.binding.type === "computer_use") {
    const displayLabel = method.binding.display_id.trim()
      ? `，显示器 ${method.binding.display_id}`
      : "";
    return `Computer use · ${method.binding.action}${displayLabel}`;
  }
  const url = method.binding.url.trim() || "未填写 URL";
  return `${method.binding.http_method || "HTTP"} ${url}`;
}

export function defaultCapabilityArgumentsText(method: UiMethodConfig): string {
  try {
    const schema = parseJson(method.input_schema_text);
    return prettyJson(sampleJsonValue(schema, method.name));
  } catch {
    if (method.binding.type === "shell_command") {
      return prettyJson({
        command: ["echo", "bridge-agent local test"]
      });
    }
    return "{}";
  }
}
