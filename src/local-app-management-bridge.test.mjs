import { readFileSync } from "node:fs";
import vm from "node:vm";
import { afterEach, expect, it, vi } from "vitest";

const source = readFileSync(new URL("../src-tauri/src/desktop/local_app_ui_bridge.js", import.meta.url), "utf8");
function bridge() {
  const listeners = new Map();
  const sent = [];
  const parent = { postMessage: (message) => sent.push(message) };
  const window = {
    parent,
    addEventListener: (name, callback) => listeners.set(name, callback),
    setTimeout,
    baijimuLocalApp: undefined,
  };
  vm.runInNewContext(source, { window, clearTimeout, setTimeout });
  return { window, sent, reply: (data) => listeners.get("message")?.({ source: parent, data }) };
}
afterEach(() => vi.useRealTimers());

it("preserves the application error code and state through the iframe bridge", async () => {
  const context = bridge();
  const result = context.window.baijimuLocalApp.invoke("credentialState");
  context.reply({ type: "baijimu:local-app:response", version: 1,
    requestId: context.sent.at(-1)?.requestId, ok: false, error: "正在提交",
    errorDetails: { code: "STATE_COMMITTING", data: { retryable: true } } });
  await expect(result).rejects.toMatchObject({ message: "正在提交", code: "STATE_COMMITTING", data: { retryable: true } });
});

it("reports timeout without claiming that the application operation was canceled", async () => {
  vi.useFakeTimers();
  const context = bridge();
  const result = context.window.baijimuLocalApp.invoke("activate");
  const assertion = expect(result).rejects.toMatchObject({ code: "connector_management_timeout", message: expect.stringContaining("可能仍在执行") });
  await vi.advanceTimersByTimeAsync(65000);
  await assertion;
});
