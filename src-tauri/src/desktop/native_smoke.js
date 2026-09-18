(() => {
  const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
  const started = Date.now();
  let busy = false;
  let completed = false;
  const finish = (success, detail) => {
    completed = true;
    return invoke("plugin:event|emit", { event: "desktop-smoke-result", payload: { success, detail } });
  };
  const timer = setInterval(async () => {
    if (busy || completed) return;
    if (Date.now() - started > 25000) {
      clearInterval(timer);
      await finish(false, "native WebView did not render the populated desktop");
      return;
    }
    if (!document.querySelector(".desktop-sidebar")) return;
    busy = true;
    try {
      const config = await invoke("load_config");
      if (!config.config.services.length) throw new Error("smoke requires nonempty config");
      const health = await invoke("get_startup_health");
      if (!health.frontendReady) { busy = false; return; }
      const panel = document.getElementById("desktop-recovery");
      const root = document.getElementById("root");
      if (!panel.hidden || root.hidden || !root.getBoundingClientRect().height) throw new Error("populated desktop is hidden by recovery");
      window.dispatchEvent(new ErrorEvent("error", { error: new Error("native nonfatal error injection") }));
      if (!panel.hidden || root.hidden) throw new Error("nonfatal error hid the desktop");
      await verifyNativeLocalAppUi(invoke);
      root.replaceChildren();
      await new Promise((resolve) => setTimeout(resolve, 3500));
      if (panel.hidden || !panel.getBoundingClientRect().height) throw new Error("recovery panel is not visible");
      if (!document.getElementById("root").hidden) throw new Error("failed business root remains visible");
      document.getElementById("recovery-check").click();
      // Missing update configuration is intentional in this isolated development executable.
      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (panel.hidden || !document.getElementById("recovery-status").textContent) throw new Error("update failure removed recovery controls");
      if ((await invoke("get_startup_health")).frontendReady) throw new Error("render failure was not persisted");
      await finish(true, "native WebView: populated desktop, recovery after fault, real update IPC failure, custom-protocol UI and origin isolation");
    } catch (error) { await finish(false, String(error)); }
    clearInterval(timer);
  }, 200);
})();

async function verifyNativeLocalAppUi(invoke) {
  const urls = await Promise.all(["native-ui-first", "native-ui-second"].map((id) => invoke("connector_app_ui_url", { id })));
  const origins = urls.map((url) => new URL(url).origin);
  if (origins[0] === origins[1] || origins.includes("null")) throw new Error("local app origins are not isolated");
  const frames = urls.map(() => document.createElement("iframe"));
  frames.forEach((frame) => { frame.sandbox = "allow-forms allow-same-origin allow-scripts"; });
  const completed = new Set();
  let listener;
  try {
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error("native local app UI handshake timed out")), 10000);
      const fail = (message) => { clearTimeout(timeout); reject(new Error(message)); };
      listener = (event) => {
        const index = frames.findIndex((frame) => event.source === frame.contentWindow);
        if (index < 0) return;
        if (event.origin !== origins[index]) { fail("native UI origin mismatch"); return; }
        if (event.data?.type === "baijimu:local-app:invoke") {
          if (event.data.operation !== "smoke") { fail("unexpected management operation"); return; }
          event.source.postMessage({ type: "baijimu:local-app:response", version: 1, requestId: event.data.requestId, ok: true, data: {} }, origins[index]);
        }
        if (event.data?.type === "native-ui-smoke-failed") fail(event.data.error);
        if (event.data?.type === "native-ui-smoke-complete") {
          if (!event.data.parentBlocked || !event.data.nativeIpcBlocked || event.data.bridgeVersion !== 1) {
            fail("native local app isolation or bridge contract failed"); return;
          }
          completed.add(index);
          if (completed.size === frames.length) { clearTimeout(timeout); resolve(); }
        }
      };
      window.addEventListener("message", listener);
      frames.forEach((frame, index) => { frame.src = urls[index]; document.body.appendChild(frame); });
    });
  } finally {
    window.removeEventListener("message", listener);
    frames.forEach((frame) => frame.remove());
  }
}
