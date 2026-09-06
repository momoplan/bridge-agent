import {
  deriveDeviceAuthorizationState,
  deviceAuthorizationLocksCapabilities
} from "../device-authorization-state";
import type { AppUpdateProgress, CommandError, ConnectorLifecycleResult, DesktopPermissionStatus, LocalAppLifecycle, LocalAppLifecycleState, RegisteredServiceState, RegisteredServiceStatus, RuntimeLockConflict, RuntimeSnapshot, RuntimeStatus, UiAgentConfig, UiServiceConfig } from "./types";
export function sampleJsonValue(schema: unknown, propertyName = ""): unknown {
  if (!schema || typeof schema !== "object") {
    return {};
  }
  const candidate = schema as {
    type?: unknown;
    enum?: unknown;
    required?: unknown;
    properties?: unknown;
    items?: unknown;
    minimum?: unknown;
    minItems?: unknown;
  };
  if (Array.isArray(candidate.enum) && candidate.enum.length > 0) {
    return candidate.enum[0];
  }

  const type = Array.isArray(candidate.type) ? candidate.type[0] : candidate.type;
  if (type === "object" || candidate.properties) {
    const properties =
      candidate.properties && typeof candidate.properties === "object"
        ? (candidate.properties as Record<string, unknown>)
        : {};
    const required = Array.isArray(candidate.required)
      ? candidate.required.filter((item): item is string => typeof item === "string")
      : Object.keys(properties);
    const result: Record<string, unknown> = {};
    for (const key of required) {
      if (properties[key]) {
        result[key] = sampleJsonValue(properties[key], key);
      }
    }
    return result;
  }

  if (type === "array") {
    if (propertyName === "command") {
      return ["echo", "bridge-agent local test"];
    }
    if (propertyName === "keys") {
      return ["Enter"];
    }
    if (propertyName === "path") {
      return [
        { x: 100, y: 100 },
        { x: 160, y: 160 }
      ];
    }
    return [sampleJsonValue(candidate.items, propertyName)];
  }

  if (type === "integer") {
    if (propertyName === "ms") {
      return 500;
    }
    return typeof candidate.minimum === "number" ? candidate.minimum : 1;
  }
  if (type === "number") {
    if (propertyName === "x" || propertyName === "y") {
      return 100;
    }
    return typeof candidate.minimum === "number" ? candidate.minimum : 1;
  }
  if (type === "boolean") {
    return true;
  }
  if (type === "string") {
    if (propertyName === "text") {
      return "本地测试";
    }
    return "";
  }
  return {};
}

export function formatDesktopPermissionValue(
  status: DesktopPermissionStatus | null,
  permission: "screen_recording" | "accessibility",
  unsupportedLabel: string
): string {
  if (!status) {
    return "检查中";
  }
  const supported =
    permission === "screen_recording" ? status.screenRecordingSupported : status.accessibilitySupported;
  if (!supported) {
    return `当前平台未接入，${unsupportedLabel}`;
  }
  if (permission === "screen_recording") {
    return status.screenRecordingGranted ? "已授权" : "未授权";
  }
  return status.accessibilityGranted ? "已授权" : "未授权";
}

export function formatRegisteredServiceDetail(
  service: UiServiceConfig,
  status: RegisteredServiceStatus | undefined
): string {
  const hasStartCommand = status?.startCommandConfigured ?? service.start_command != null;
  const hasStopCommand = status?.stopCommandConfigured ?? service.stop_command != null;
  const hasHealthCheck = status?.healthCheckConfigured ?? service.health_check != null;
  if (!hasHealthCheck && !hasStartCommand && !hasStopCommand) {
    return "仅作为能力清单展示";
  }
  if (!hasHealthCheck && hasStartCommand) {
    return "已配置启动命令；未配置 healthCheck，无法自动确认运行状态";
  }
  return status?.detail ?? "等待状态检查";
}

export function formatRegisteredServiceStatus(status: RegisteredServiceState): string {
  const labels: Record<RegisteredServiceState, string> = {
    not_configured: "未配置",
    healthy: "可用",
    unhealthy: "不可用",
    unknown: "未知"
  };
  return labels[status];
}

export function formatLocalAppLifecycle(
  state: LocalAppLifecycleState,
  detail?: string
): LocalAppLifecycle {
  const labels: Record<LocalAppLifecycleState, string> = {
    absent: "未安装",
    installing: "安装中",
    stopped: "已停止",
    starting: "启动中",
    ready: "可用",
    degraded: "运行异常",
    stopping: "停止中",
    upgrading: "升级中",
    uninstalling: "卸载中",
    recovering: "恢复中",
    failed: "操作失败"
  };
  const details: Record<LocalAppLifecycleState, string> = {
    absent: "应用尚未安装",
    installing: "正在后台安装应用",
    stopped: "应用已停止",
    starting: "正在执行应用启动命令",
    ready: "应用已启动并通过就绪检查",
    degraded: "应用进程存在，但健康检查未通过",
    stopping: "正在执行应用停止命令",
    upgrading: "正在安装或切换版本",
    uninstalling: "正在停止并卸载应用",
    recovering: "正在核对并恢复应用运行状态",
    failed: "生命周期操作失败"
  };
  const statusClasses: Record<LocalAppLifecycleState, string> = {
    absent: "stopped",
    installing: "starting",
    stopped: "stopped",
    starting: "starting",
    ready: "running",
    degraded: "start_failed",
    stopping: "stopping",
    upgrading: "starting",
    uninstalling: "stopping",
    recovering: "unknown",
    failed: "start_failed"
  };
  return {
    state,
    label: labels[state],
    detail: detail ?? details[state],
    statusClass: statusClasses[state]
  };
}

export function compareVersions(left: string, right: string): number {
  const parse = (value: string) => {
    const [core, prerelease = ""] = value.trim().replace(/^v/, "").split("-", 2);
    const parts = core.split(".").map((part) => Number.parseInt(part, 10) || 0);
    return { parts, prerelease };
  };
  const leftVersion = parse(left);
  const rightVersion = parse(right);
  const length = Math.max(leftVersion.parts.length, rightVersion.parts.length);
  for (let index = 0; index < length; index += 1) {
    const difference = (leftVersion.parts[index] ?? 0) - (rightVersion.parts[index] ?? 0);
    if (difference !== 0) {
      return difference > 0 ? 1 : -1;
    }
  }
  if (leftVersion.prerelease === rightVersion.prerelease) {
    return 0;
  }
  if (!leftVersion.prerelease) {
    return 1;
  }
  if (!rightVersion.prerelease) {
    return -1;
  }
  return leftVersion.prerelease.localeCompare(rightVersion.prerelease);
}

export function formatConnectorServiceFailures(results: ConnectorLifecycleResult[]): string {
  return results
    .map((result) => `${result.appId}${result.stderr.trim() ? ` ${result.stderr.trim()}` : ""}`)
    .join("；");
}

export function headersToText(headers: Record<string, string>): string {
  return Object.entries(headers)
    .map(([key, value]) => `${key}: ${value}`)
    .join("\n");
}

export function textToHeaders(value: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const line of value.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }
    const separator = trimmed.indexOf(":");
    if (separator <= 0) {
      throw new Error(`无效请求头: ${trimmed}`);
    }
    const key = trimmed.slice(0, separator).trim();
    const headerValue = trimmed.slice(separator + 1).trim();
    result[key] = headerValue;
  }
  return result;
}

export function parseJson(text: string): unknown {
  if (hasSmartJsonQuotes(text)) {
    throw new Error('JSON 格式错误：请使用英文双引号 "，不要使用中文/智能引号 “ ”。');
  }
  return JSON.parse(text);
}

export function prettyJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function hasSmartJsonQuotes(text: string): boolean {
  return /[“”]/.test(text);
}

export function serviceSignature(service: UiServiceConfig): string {
  return JSON.stringify(service);
}

export function safeNumber(value: string, fallback: number): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export function toOptionalText(value?: number | null): string {
  return value == null ? "" : String(value);
}

export function toOptionalNumber(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) {
    throw new Error(`无效数字: ${value}`);
  }
  return parsed;
}

export function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleString("zh-CN", {
    hour12: false
  });
}

export function formatPublishedAt(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return new Date(timestamp).toLocaleDateString("zh-CN");
}

export function formatRelayRegistration(snapshot: RuntimeSnapshot | null): string {
  if (!snapshot) {
    return "-";
  }
  if (!snapshot.relay_registered) {
    return "未注册";
  }
  return snapshot.relay_registered_at
    ? `已注册 ${formatEpochSeconds(snapshot.relay_registered_at)}`
    : "已注册";
}

export function formatRelaySeen(snapshot: RuntimeSnapshot | null): string {
  if (!snapshot?.last_relay_seen_at) {
    return "-";
  }
  return formatTime(snapshot.last_relay_seen_at);
}

export function formatEpochSeconds(timestamp: number): string {
  return formatTime(timestamp * 1000);
}

export function calculateAppUpdateProgressPercent(progress: AppUpdateProgress | null): number | null {
  if (progress?.downloadedBytes == null || !progress.totalBytes || progress.totalBytes <= 0) {
    if (progress?.phase === "ready_to_install") {
      return 100;
    }
    return null;
  }
  return Math.max(0, Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100)));
}

export function formatAppUpdateProgressButton(progress: AppUpdateProgress | null): string {
  const percent = calculateAppUpdateProgressPercent(progress);
  if (percent != null && progress?.phase === "downloading") {
    return `下载中 ${percent}%`;
  }
  switch (progress?.phase) {
    case "checking":
      return "检查中";
    case "verifying":
      return "校验中";
    case "installing":
      return "安装中";
    case "saving":
      return "保存中";
    case "scheduling":
      return "准备安装";
    case "ready_to_install":
      return "即将重启";
    default:
      return "准备更新";
  }
}

export function formatAppUpdateProgressDetail(progress: AppUpdateProgress): string {
  const sizeText =
    progress.downloadedBytes == null
      ? null
      : progress.totalBytes
        ? `${formatByteSize(progress.downloadedBytes)} / ${formatByteSize(progress.totalBytes)}`
        : `${formatByteSize(progress.downloadedBytes)} 已下载`;
  const parts = [
    progress.assetName ? `更新包 ${progress.assetName}` : null,
    sizeText,
    progress.downloadedPath && progress.phase !== "downloading" ? `保存到 ${progress.downloadedPath}` : null
  ].filter((part): part is string => Boolean(part));
  return parts.length > 0 ? parts.join("，") : "正在连接更新服务，请稍候。";
}

export function formatStartupComponentStatus(status: string): string {
  const labels: Record<string, string> = {
    starting: "启动中",
    ready: "正常",
    unavailable: "暂不可用",
    degraded: "异常",
    skipped: "已跳过"
  };
  return labels[status] ?? status;
}

export function formatByteSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value >= 10 || unitIndex === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unitIndex]}`;
}

export function needsBrowserAuthorization(
  config: UiAgentConfig,
  runtime?: Pick<RuntimeSnapshot, "status"> | null
): boolean {
  return deviceAuthorizationLocksCapabilities(deriveDeviceAuthorizationState({
    workspaceId: config.platform.workspace_id,
    relayTokenConfigured: config.credential_status.relay_token_configured,
    runtimeStatus: runtime?.status
  }));
}

export function formatStartAgentMessage(snapshot: RuntimeSnapshot): string {
  const messages: Partial<Record<RuntimeStatus, string>> = {
    starting: "Agent 正在启动",
    connecting: "Agent 正在连接",
    backoff: "Agent 正在重连等待",
    authorization_required: "Agent 需要重新授权",
    online: "Agent 已启动"
  };
  return messages[snapshot.status] ?? "Agent 已启动";
}

export function readError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (isCommandError(error)) {
    if (error.code === "runtime_already_running") {
      return `百积木已经在运行，PID ${error.conflict.pid} 正在占用当前配置。`;
    }
    return error.message;
  }
  return String(error);
}

export function readRuntimeConflict(error: unknown): RuntimeLockConflict | null {
  if (isCommandError(error) && error.code === "runtime_already_running") {
    return error.conflict;
  }
  return null;
}

export function isCommandError(error: unknown): error is CommandError {
  if (!error || typeof error !== "object" || !("code" in error)) {
    return false;
  }
  const candidate = error as { code?: unknown; conflict?: unknown; message?: unknown };
  if (candidate.code === "message") {
    return typeof candidate.message === "string";
  }
  if (
    candidate.code === "connector_uninstall_stop_failed" ||
    candidate.code === "connector_uninstall_failed" ||
    candidate.code === "connector_not_ready" ||
    candidate.code === "connector_management_failed"
  ) {
    return typeof candidate.message === "string";
  }
  if (candidate.code !== "runtime_already_running") {
    return false;
  }
  const conflict = candidate.conflict as Partial<RuntimeLockConflict> | undefined;
  return Boolean(
    conflict &&
      typeof conflict.pid === "number" &&
      typeof conflict.agent_id === "string" &&
      typeof conflict.config_path === "string" &&
      typeof conflict.lock_path === "string" &&
      conflict.process &&
      typeof conflict.process === "object"
  );
}
