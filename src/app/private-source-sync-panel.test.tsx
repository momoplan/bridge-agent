// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { PrivateSourceSyncPanel } from "./private-source-sync-panel";
import type { LocalAppItem } from "./types";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: ReturnType<typeof createRoot> | undefined;
afterEach(async () => { await act(async () => root?.unmount()); document.body.innerHTML = ""; });
const app = { id: "fixture", name: "Fixture", connector: { appId: "fixture-app", version: "1.2.3", installSource: { kind: "environment", source: { application: { environmentKey: "author-source", appId: "fixture-app" }, version: "1.2.3" } } } } as LocalAppItem;

it("shows the source, accepts a target version and submits that exact version", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  root = createRoot(document.getElementById("root")!);
  const sync = vi.fn(async () => {});
  await act(async () => root!.render(<PrivateSourceSyncPanel app={app} busy={false} syncLocalApp={sync} />));
  expect(document.body.textContent).toContain("author-source / fixture-app");
  const input = document.querySelector("input")!;
  expect(input.value).toBe("1.2.3");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "1.2.4");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(sync).toHaveBeenCalledWith(app, "1.2.4");
});

it("blocks submission without a saved environment identity", async () => {
  document.body.innerHTML = '<div id="root"></div>';
  root = createRoot(document.getElementById("root")!);
  const missing = { ...app, connector: { ...app.connector!, installSource: null } };
  await act(async () => root!.render(<PrivateSourceSyncPanel app={missing} busy={false} syncLocalApp={vi.fn()} />));
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("缺少来源环境身份");
  expect(document.querySelector("form")).toBeNull();
});
