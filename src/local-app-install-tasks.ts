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

export function reconcileLocalAppInstallSelection(
  currentSelection: string | null,
  availableLocalAppIds: Iterable<string>,
  tasks: LocalAppInstallTask[]
): string | null {
  if (currentSelection == null) {
    return null;
  }

  const available = new Set(availableLocalAppIds);
  if (available.has(currentSelection)) {
    return currentSelection;
  }

  const taskPrefix = "install-task:";
  if (!currentSelection.startsWith(taskPrefix)) {
    return null;
  }

  const taskId = currentSelection.slice(taskPrefix.length);
  const task = tasks.find((candidate) => candidate.taskId === taskId);
  if (task?.state !== "succeeded" || !task.connectorId) {
    return null;
  }

  const installedAppId = `connector:${task.connectorId}`;
  return available.has(installedAppId) ? installedAppId : null;
}

export function shouldShowLocalAppInstallTask(
  task: LocalAppInstallTask,
  installedConnectorIds: Iterable<string>
): boolean {
  return !task.connectorId || !new Set(installedConnectorIds).has(task.connectorId);
}
