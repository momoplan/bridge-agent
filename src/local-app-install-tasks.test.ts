import { describe, expect, it } from "vitest";
import {
  latestLocalAppInstallTasks,
  reconcileLocalAppInstallSelection,
  shouldShowLocalAppInstallTask,
  type LocalAppInstallTask
} from "./local-app-install-tasks";

function task(
  taskId: string,
  state: LocalAppInstallTask["state"],
  updatedAtEpochMs: number
): LocalAppInstallTask {
  return {
    taskId,
    connectorId: "com.baijimu.connector.codex",
    marketAppId: "codex",
    name: "Codex",
    state,
    message: state,
    createdAtEpochMs: updatedAtEpochMs,
    updatedAtEpochMs
  };
}

describe("local app install task selection", () => {
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
