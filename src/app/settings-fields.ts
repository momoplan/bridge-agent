import { fromUiConfig } from "./config-conversion";
import type { UiAgentConfig } from "./types";

export const settingsSections = [
  { id: "general", label: "通用", description: "管理客户端的启动行为。" },
  { id: "identity", label: "设备与账号", description: "管理设备信息与账号授权。" },
  { id: "connection", label: "连接", description: "管理平台连接和上传参数。" },
  { id: "runtime", label: "运行", description: "管理本机解释器、超时和日志。" },
  { id: "events", label: "事件与存储", description: "管理事件重试和存储策略。" },
  { id: "about", label: "关于与更新", description: "查看客户端版本与更新。" },
] as const;
export type SettingsPageId = typeof settingsSections[number]["id"];
export type EditableSettingsPage = Exclude<SettingsPageId, "general" | "about">;
const fields: Record<EditableSettingsPage, string[]> = {
  identity: ["device.name", "device.description", "device.tags", "relay.agent_id"],
  connection: ["platform.base_url", "relay.url", "relay.reconnect_secs", "upload.prepare_url", "upload.inline_limit_bytes", "upload.timeout_secs"],
  events: ["runtime.event_delivery"],
  runtime: ["runtime.python_path", "runtime.node_path", "runtime.default_timeout_secs", "runtime.max_timeout_secs", "runtime.log_limit", "runtime.log_file_enabled", "runtime.log_file_dir", "runtime.log_file_max_bytes", "runtime.log_file_max_files", "runtime.event_server_enabled", "runtime.event_server_bind"],
};
export function settingsValues(config: UiAgentConfig, section: SettingsPageId): Record<string, unknown> {
  if (section === "general" || section === "about") return {};
  // Only convert the editable document sections; unrelated service drafts may be incomplete.
  const document = fromUiConfig({ ...config, services: [] }) as unknown as Record<string, Record<string, unknown>>;
  return Object.fromEntries(fields[section].map(path => {
    const [group, key] = path.split(".");
    return [path, document[group][key] ?? null];
  }));
}
export function settingsChanges(original: UiAgentConfig, draft: UiAgentConfig, section: SettingsPageId) {
  const before = settingsValues(original, section);
  return Object.fromEntries(Object.entries(settingsValues(draft, section)).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(before[key])));
}
