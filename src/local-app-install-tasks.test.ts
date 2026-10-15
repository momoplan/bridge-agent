import { describe, expect, it } from "vitest";
import {
  formatLocalAppInstallTaskPhase,
  latestLocalAppInstallTasks,
  mergeLocalAppInstallTasks,
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
    revision: 1,
    operation: "install",
    appId: "com.baijimu.connector.codex",
    name: "Codex",
    phase,
    message: phase,
    createdAtEpochMs: updatedAtEpochMs,
    updatedAtEpochMs
  };
}

describe("local app install task selection", () => {
  it("preserves failure when the initial command response arrives after progress", () => {
    const queued = task("same", "queued", 10);
    const resolving = { ...queued, phase: "resolving" as const, revision: 2 };
    const failed = { ...queued, phase: "failed" as const, revision: 3, error: "来源版本不可用" };
    const progress = mergeLocalAppInstallTasks([], [resolving, failed]);
    expect(mergeLocalAppInstallTasks(progress, [queued])).toEqual([failed]);
    expect(mergeLocalAppInstallTasks([queued], [resolving, failed])).toEqual([failed]);
  });

  it("orders equal-millisecond snapshots and progress by owner revision", () => {
    const queued = task("same", "queued", 10);
    const downloading = { ...queued, phase: "downloading" as const, revision: 4 };
    expect(mergeLocalAppInstallTasks([downloading], [queued, { ...queued, revision: 3 }])).toEqual([downloading]);
    expect(mergeLocalAppInstallTasks([queued], [downloading])).toEqual([downloading]);
  });

  it("keeps terminal states and lets a distinct retry task progress", () => {
    const failed = { ...task("old", "failed", 10), revision: 4 };
    const retry = { ...task("new", "downloading", 20), revision: 2 };
    expect(mergeLocalAppInstallTasks([failed], [{ ...failed, revision: 5, phase: "resolving" }, retry])).toEqual([failed, retry]);
    expect(latestLocalAppInstallTasks([{ ...failed, updatedAtEpochMs: 30 }, retry])).toEqual([retry]);
  });
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
    const first = { ...task("first", "resolving", 10), appId: null };
    const second = { ...task("second", "resolving", 20), appId: null };

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
