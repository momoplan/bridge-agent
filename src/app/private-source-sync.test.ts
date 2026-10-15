// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { privateSourceSyncRequest } from "./private-source-sync";
import { createLocalAppCatalogActions } from "./local-app-catalog-actions";
import { createLocalAppLifecycleActions } from "./local-app-lifecycle-actions";
import type { AppControllerState } from "./use-app-state";
import type { ConnectorSummary, LocalAppItem } from "./types";
import type { LocalAppInstallTask } from "../local-app-install-tasks";

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: ipc.invoke, Channel: class { onmessage?: (task: LocalAppInstallTask) => void } }));

function connector(): ConnectorSummary {
  return {
    appId: "fixture-app", name: "Fixture", version: "1.2.3", reviewStatus: "SOURCE_FROZEN",
    installSource: { kind: "environment", source: { application: { environmentKey: "author-source", appId: "fixture-app" }, version: "1.2.3" } }
  } as ConnectorSummary;
}

beforeEach(() => { ipc.invoke.mockReset(); });

describe("private source version selection", () => {
  it("preserves the source application and explicitly selects the requested version", () => {
    const installed = connector();
    expect(privateSourceSyncRequest(installed, "1.2.4+signed")).toMatchObject({
      installSource: { kind: "environment", source: { application: { environmentKey: "author-source", appId: "fixture-app" }, version: "1.2.4+signed" } },
      version: "1.2.4+signed", appId: "fixture-app", sourceIdentityMigration: false, replace: true, acceptUnreviewed: true
    });
    expect(installed.installSource?.source.version).toBe("1.2.3");
    expect(privateSourceSyncRequest(installed, "1.2.3").version).toBe("1.2.3");
  });

  it("rejects missing or mismatched identity without inferring from temporary paths", () => {
    expect(() => privateSourceSyncRequest({ ...connector(), installSource: null, sourcePath: "/tmp/old/package" }, "1.2.4")).toThrow("来源环境身份");
    const other = connector();
    other.appId = "another-app";
    expect(() => privateSourceSyncRequest(other, "1.2.4")).toThrow("来源应用身份不一致");
    expect(() => privateSourceSyncRequest({ ...connector(), installSource: { kind: "market", listingId: "listing", version: "1.2.3", source: connector().installSource!.source } }, "1.2.4")).toThrow("来源环境身份");
  });

  it.each(["latest", "1.2", "01.2.3", "1.2.3-01", " 1.2.4", "1.2.4 ", "1.2.4\n"])("rejects noncanonical version %s", (version) => {
    expect(() => privateSourceSyncRequest(connector(), version)).toThrow("SemVer");
  });
});

describe("private sync command and progress integration", () => {
  it.each([true, false])("keeps the real failure when progress arrives before command response: %s", async (early) => {
    let tasks: LocalAppInstallTask[] = [];
    const handleCommandError = vi.fn();
    const state = new Proxy({
      marketConnectors: [],
      setLocalAppInstallTasks: (update: (previous: LocalAppInstallTask[]) => LocalAppInstallTask[]) => { tasks = update(tasks); }
    }, { get(target, key) { return key in target ? target[key as keyof typeof target] : vi.fn(); } }) as unknown as AppControllerState;
    const catalog = createLocalAppCatalogActions(state, {} as Parameters<typeof createLocalAppCatalogActions>[1]);
    const lifecycle = createLocalAppLifecycleActions(state, { startLocalAppInstallTask: catalog.startLocalAppInstallTask, handleCommandError } as unknown as Parameters<typeof createLocalAppLifecycleActions>[1]);
    const app = { id: "connector:fixture-app", name: "Fixture", connector: connector() } as LocalAppItem;
    const queued: LocalAppInstallTask = { taskId: "task", revision: 1, operation: "sync", appId: "fixture-app", name: "Fixture", version: "1.2.4", phase: "queued", progressPercent: 0, message: "等待开始同步", createdAtEpochMs: 1, updatedAtEpochMs: 1 };
    const resolving: LocalAppInstallTask = { ...queued, revision: 2, phase: "resolving" };
    const failed: LocalAppInstallTask = { ...queued, revision: 3, phase: "failed", error: "来源版本未授权", message: "应用同步失败" };
    let channel: { onmessage: (task: LocalAppInstallTask) => void };
    ipc.invoke.mockImplementation(async (_name, args) => {
      channel = args.onEvent;
      if (early) { channel.onmessage(resolving); channel.onmessage(failed); }
      return queued;
    });
    vi.spyOn(window, "confirm").mockReturnValueOnce(true);
    await lifecycle.syncLocalApp(app, "1.2.4");
    if (!early) { channel!.onmessage(resolving); channel!.onmessage(failed); }
    expect(ipc.invoke).toHaveBeenCalledWith("start_connector_app_install", expect.objectContaining({ request: expect.objectContaining({ version: "1.2.4", installSource: { kind: "environment", source: { application: { environmentKey: "author-source", appId: "fixture-app" }, version: "1.2.4" } } }) }));
    expect(tasks).toEqual([failed]);
    expect(handleCommandError).not.toHaveBeenCalled();
  });
});
