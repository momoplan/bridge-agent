import { describe, expect, it } from "vitest";
import {
  formatLocalAppInstallTaskPhase,
  latestLocalAppInstallTasks,
  reconcileLocalAppInstallSelection,
  shouldShowLocalAppInstallTask,
  type LocalAppInstallTask
} from "./local-app-install-tasks";

function task(
  taskId: string,
  phase: LocalAppInstallTask["phase"],
  updatedAtEpochMs: number
): LocalAppInstallTask {
  return {
    taskId,
    operation: "install",
    connectorId: "com.baijimu.connector.codex",
    marketAppId: "codex",
    name: "Codex",
    phase,
    message: phase,
    createdAtEpochMs: updatedAtEpochMs,
    updatedAtEpochMs
  };
}

describe("local app install task selection", () => {
  it("uses operation-specific progress labels", () => {
    expect(formatLocalAppInstallTaskPhase(task("install", "downloading", 10))).toBe("下载安装包");
    expect(
      formatLocalAppInstallTaskPhase({
        ...task("upgrade", "downloading", 10),
        operation: "upgrade"
      })
    ).toBe("下载升级包");
    expect(
      formatLocalAppInstallTaskPhase({
        ...task("sync", "succeeded", 10),
        operation: "sync"
      })
    ).toBe("同步完成");
  });

  it("keeps only the latest attempt for the same application", () => {
    expect(
      latestLocalAppInstallTasks([
        task("failed", "failed", 10),
        task("retry", "downloading", 20)
      ])
    ).toEqual([task("retry", "downloading", 20)]);
  });

  it("keeps unrelated custom-source tasks independent before identity resolution", () => {
    const first = { ...task("first", "resolving", 10), connectorId: null, marketAppId: null };
    const second = { ...task("second", "resolving", 20), connectorId: null, marketAppId: null };

    expect(latestLocalAppInstallTasks([first, second])).toEqual([first, second]);
  });

  it("keeps the visible install task selected until the installed app reaches the catalog", () => {
    const completed = task("install-codex", "succeeded", 20);

    expect(
      reconcileLocalAppInstallSelection(
        "install-task:install-codex",
        ["install-task:install-codex"],
        [completed]
      )
    ).toBe("install-task:install-codex");
  });

  it("opens the installed connector when the catalog catches up with a successful task", () => {
    const completed = task("install-codex", "succeeded", 20);

    expect(
      reconcileLocalAppInstallSelection(
        "install-task:install-codex",
        ["connector:com.baijimu.connector.codex"],
        [completed]
      )
    ).toBe("connector:com.baijimu.connector.codex");
  });

  it("keeps a successful task visible only until its connector reaches the catalog", () => {
    const completed = task("install-codex", "succeeded", 20);

    expect(shouldShowLocalAppInstallTask(completed, [])).toBe(true);
    expect(
      shouldShowLocalAppInstallTask(completed, ["com.baijimu.connector.codex"])
    ).toBe(false);
  });

  it("does not redirect a failed or unrelated selection", () => {
    expect(
      reconcileLocalAppInstallSelection(
        "install-task:install-codex",
        ["connector:com.baijimu.connector.codex"],
        [task("install-codex", "failed", 20)]
      )
    ).toBeNull();
    expect(
      reconcileLocalAppInstallSelection(
        "built-in:shell",
        ["connector:com.baijimu.connector.codex"],
        [task("install-codex", "succeeded", 20)]
      )
    ).toBeNull();
  });
});
