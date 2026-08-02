import { describe, expect, it } from "vitest";
import {
  latestLocalAppInstallTasks,
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
});
