import { describe, expect, it, vi } from "vitest";
import { loadSynchronizedLocalAppCatalog } from "./local-app-catalog";

describe("local app catalog loading", () => {
  it("synchronizes installed apps before reading the config snapshot", async () => {
    const calls: string[] = [];
    const listInstalledApps = vi.fn(async () => {
      calls.push("list");
      return [{ id: "com.baijimu.connector.wecom" }];
    });
    const loadConfig = vi.fn(async () => {
      calls.push("config");
      return { localApps: ["com.baijimu.connector.wecom"] };
    });

    const result = await loadSynchronizedLocalAppCatalog(listInstalledApps, loadConfig);

    expect(calls).toEqual(["list", "config"]);
    expect(result.apps).toEqual([{ id: "com.baijimu.connector.wecom" }]);
    expect(result.document).toEqual({ localApps: ["com.baijimu.connector.wecom"] });
  });

  it("does not read config when installed app synchronization fails", async () => {
    const loadConfig = vi.fn(async () => ({ localApps: [] }));

    await expect(
      loadSynchronizedLocalAppCatalog(
        async () => {
          throw new Error("sync failed");
        },
        loadConfig
      )
    ).rejects.toThrow("sync failed");
    expect(loadConfig).not.toHaveBeenCalled();
  });
});
