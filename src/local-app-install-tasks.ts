export type LocalAppInstallTaskPhase =
  | "queued"
  | "resolving"
  | "downloading"
  | "verifying"
  | "installing"
  | "starting"
  | "finalizing"
  | "succeeded"
  | "failed";

export type LocalAppInstallTaskOperation = "install" | "upgrade" | "sync";

export interface LocalAppInstallTask {
  taskId: string;
  operation: LocalAppInstallTaskOperation;
  appId?: string | null;
  name: string;
  version?: string | null;
  phase: LocalAppInstallTaskPhase;
  progressPercent?: number | null;
  downloadedBytes?: number | null;
  totalBytes?: number | null;
  message: string;
  error?: string | null;
  createdAtEpochMs: number;
  updatedAtEpochMs: number;
}

export function formatLocalAppInstallTaskPhase(task: LocalAppInstallTask): string {
  const operationLabels: Record<
    LocalAppInstallTaskOperation,
    Record<LocalAppInstallTaskPhase, string>
  > = {
    install: {
      queued: "等待安装",
      resolving: "解析来源",
      downloading: "下载安装包",
      verifying: "校验安装包",
      installing: "安装应用",
      starting: "启动应用",
      finalizing: "刷新应用能力",
      succeeded: "安装完成",
      failed: "安装失败"
    },
    upgrade: {
      queued: "等待升级",
      resolving: "解析升级来源",
      downloading: "下载升级包",
      verifying: "校验升级包",
      installing: "安装新版本",
      starting: "启动新版本",
      finalizing: "刷新应用能力",
      succeeded: "升级完成",
      failed: "升级失败"
    },
    sync: {
      queued: "等待同步",
      resolving: "解析同步来源",
      downloading: "下载来源包",
      verifying: "校验来源包",
      installing: "同步应用",
      starting: "启动应用",
      finalizing: "刷新应用能力",
      succeeded: "同步完成",
      failed: "同步失败"
    }
  };
  return operationLabels[task.operation][task.phase];
}

export function latestLocalAppInstallTasks(
  tasks: LocalAppInstallTask[]
): LocalAppInstallTask[] {
  return Array.from(
    tasks.reduce((latest, task) => {
      const key = task.appId || task.taskId;
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
  if (task?.phase !== "succeeded" || !task.appId) {
    return null;
  }

  const installedAppId = `connector:${task.appId}`;
  return available.has(installedAppId) ? installedAppId : null;
}

export function shouldShowLocalAppInstallTask(
  task: LocalAppInstallTask,
  installedConnectorIds: Iterable<string>
): boolean {
  return !task.appId || !new Set(installedConnectorIds).has(task.appId);
}
