import { invoke } from "@tauri-apps/api/core";
import type { LogEntry } from "./types";

let runtimeLogStreamingCommand: Promise<void> = Promise.resolve();

export function updateRuntimeLogStreaming(enabled: boolean): Promise<void> {
  const command = runtimeLogStreamingCommand
    .catch(() => undefined)
    .then(() => invoke<void>("set_runtime_log_streaming", { enabled }));
  runtimeLogStreamingCommand = command.catch(() => undefined);
  return command;
}

export function mergeRecentLogs(...groups: LogEntry[][]) {
  const merged = new Map<number, LogEntry>();
  groups.flat().forEach((entry) => {
    merged.set(entry.sequence, entry);
  });
  return Array.from(merged.values())
    .sort((left, right) => left.sequence - right.sequence)
    .slice(-200);
}
