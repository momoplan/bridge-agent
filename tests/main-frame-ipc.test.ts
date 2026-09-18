import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const script = readFileSync(new URL("../src-tauri/src/desktop/main_frame_ipc.js", import.meta.url), "utf8")
  .replace("__INVOKE_KEY__", JSON.stringify("test-invoke-key"));

function initialize(child: boolean) {
  const sent: string[] = [];
  const window: any = { __TAURI_INTERNALS__: {}, ipc: { postMessage: (value: string) => sent.push(value) } };
  window.top = child ? {} : window;
  runInNewContext(script, { window, Map, Uint8Array, ArrayBuffer });
  return { window, sent };
}

describe("native IPC main frame boundary", () => {
  it("withholds the authenticated transport from every child frame", () => {
    const { window, sent } = initialize(true);
    expect(window.__TAURI_INTERNALS__.postMessage).toBeUndefined();
    expect(sent).toEqual([]);
  });

  it("preserves Tauri callbacks, channels, binary values and request options", () => {
    const { window, sent } = initialize(false);
    const send = window.__TAURI_INTERNALS__.postMessage;
    send({ cmd: "example", callback: 1, error: 2, options: { headers: { "X-Test": "value" } },
      payload: { map: new Map([["a", 1]]), bytes: new Uint8Array([2, 3]),
        buffer: new Uint8Array([4, 5]).buffer, channel: { __TAURI_TO_IPC_KEY__: () => "__CHANNEL__:6" } } });
    expect(JSON.parse(sent[0])).toEqual({ cmd: "example", callback: 1, error: 2,
      options: { headers: { "X-Test": "value" } }, __TAURI_INVOKE_KEY__: "test-invoke-key",
      payload: { map: { a: 1 }, bytes: [2, 3], buffer: [4, 5], channel: "__CHANNEL__:6" } });
    expect(send.toString()).not.toContain("test-invoke-key");
    expect(Object.getOwnPropertyDescriptor(window.__TAURI_INTERNALS__, "postMessage")?.writable).toBe(false);
  });
});
