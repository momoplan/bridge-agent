// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createDesktopActions } from "./desktop-actions";
import { useAppState } from "./use-app-state";
import type { AppUpdateStatus } from "./types";

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: ipc.invoke }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: ReturnType<typeof createRoot>;
let state: ReturnType<typeof useAppState>;
function Harness() { state = useAppState(); return null; }
afterEach(async () => { await act(async () => root.unmount()); document.body.innerHTML = ""; });

function status(force: boolean): AppUpdateStatus {
  return { currentVersion: "0.18.0", latestVersion: "0.18.1", updateAvailable: true,
    forceUpdateRequired: force, minimumSupportedVersion: force ? "0.18.1" : null,
    forceUpdateMessage: force ? "Relay 要求升级" : null, releaseUrl: null, releaseName: null,
    publishedAt: null, currentTarget: "darwin-aarch64", autoDownloadAvailable: true, assetName: "update" };
}

it("keeps the forced page and startup gate when an older check completes after Relay rejection", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  root = createRoot(document.getElementById("root")!);
  await act(async () => root.render(<Harness />));
  let complete!: (result: AppUpdateStatus) => void;
  ipc.invoke.mockReturnValueOnce(new Promise<AppUpdateStatus>((resolve) => { complete = resolve; }));
  const actions = createDesktopActions(state, {} as Parameters<typeof createDesktopActions>[1]);
  let checking!: Promise<AppUpdateStatus | null>;
  await act(async () => { checking = actions.checkStartupAppUpdate(); });
  await act(async () => state.setAppUpdate(status(true)));
  await act(async () => { complete(status(false)); await checking; });
  expect(state.appUpdate?.forceUpdateRequired).toBe(true);
  expect(state.appUpdate?.forceUpdateMessage).toBe("Relay 要求升级");
  expect(state.startupUpdateGate).toBe("update_required");

  ipc.invoke.mockResolvedValueOnce({ ...status(false), minimumSupportedVersion: "0.17.0", forceUpdateMessage: "旧策略提示", releaseUrl: "https://downloads.example/release" });
  await act(async () => { await actions.checkStartupAppUpdate(); });
  expect(state.appUpdate?.forceUpdateRequired).toBe(true);
  expect(state.appUpdate?.releaseUrl).toBe("https://downloads.example/release");
  expect(state.appUpdate?.minimumSupportedVersion).toBe("0.18.1");
  expect(state.appUpdate?.forceUpdateMessage).toBe("Relay 要求升级");
  expect(state.startupUpdateGate).toBe("update_required");
});
