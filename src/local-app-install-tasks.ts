export type LocalAppInstallTaskState =
  | "queued"
  | "resolving"
  | "downloading"
  | "verifying"
  | "installing"
  | "starting"
  | "finalizing"
  | "succeeded"
  | "failed";

export interface LocalAppInstallTask {
  taskId: string;
  connectorId?: string | null;
  marketAppId?: string | null;
  name: string;
  version?: string | null;
  state: LocalAppInstallTaskState;
  progressPercent?: number | null;
  downloadedBytes?: number | null;
  totalBytes?: number | null;
  message: string;
  error?: string | null;
  createdAtEpochMs: number;
  updatedAtEpochMs: number;
}

export function latestLocalAppInstallTasks(
  tasks: LocalAppInstallTask[]
): LocalAppInstallTask[] {
  return Array.from(
    tasks.reduce((latest, task) => {
      const key = task.connectorId || task.marketAppId || task.taskId;
      const current = latest.get(key);
      if (!current || current.updatedAtEpochMs < task.updatedAtEpochMs) {
        latest.set(key, task);
      }
      return latest;
    }, new Map<string, LocalAppInstallTask>()).values()
  );
}
