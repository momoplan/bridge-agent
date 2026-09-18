// Tauri v2's documented invoke_system API, using the native postMessage transport.
// WebView2 ignores the main-frame-only flag on initialization scripts.
// Keep the key in this closure so Function.toString() cannot reveal it.
(() => {
  if (window !== window.top) return;
  const invokeKey = __INVOKE_KEY__;
  const serialize = (_key, value) => {
    if (value instanceof Map) return Object.fromEntries(value.entries());
    if (value instanceof Uint8Array) return Array.from(value);
    if (value instanceof ArrayBuffer) return Array.from(new Uint8Array(value));
    if (value !== null && typeof value === "object" && "__TAURI_TO_IPC_KEY__" in value) {
      return value.__TAURI_TO_IPC_KEY__();
    }
    return value;
  };
  Object.defineProperty(window.__TAURI_INTERNALS__, "postMessage", {
    value(message) {
      const { cmd, callback, error, payload, options } = message;
      window.ipc.postMessage(JSON.stringify({ cmd, callback, error, payload, options,
        __TAURI_INVOKE_KEY__: invokeKey }, serialize));
    },
  });
})();
