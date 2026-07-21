import { error, info, warn } from "@tauri-apps/plugin-log";

function formatMessage(message: string, detail?: unknown): string {
  if (detail == null) return message;
  if (detail instanceof Error) return `${message}: ${detail.message}`;
  if (typeof detail === "string") return `${message}: ${detail}`;
  try {
    return `${message}: ${JSON.stringify(detail)}`;
  } catch {
    return `${message}: ${String(detail)}`;
  }
}

export function clientInfo(message: string, detail?: unknown): void {
  void info(formatMessage(message, detail)).catch(() => {});
}

export function clientWarn(message: string, detail?: unknown): void {
  void warn(formatMessage(message, detail)).catch(() => {});
}

export function clientError(message: string, detail?: unknown): void {
  void error(formatMessage(message, detail)).catch(() => {});
}
