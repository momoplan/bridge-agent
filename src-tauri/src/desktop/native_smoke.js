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
      root.replaceChildren();
      await new Promise((resolve) => setTimeout(resolve, 3500));
      if (panel.hidden || !panel.getBoundingClientRect().height) throw new Error("recovery panel is not visible");
      if (!document.getElementById("root").hidden) throw new Error("failed business root remains visible");
      document.getElementById("recovery-check").click();
      // Missing update configuration is intentional in this isolated development executable.
      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (panel.hidden || !document.getElementById("recovery-status").textContent) throw new Error("update failure removed recovery controls");
      if ((await invoke("get_startup_health")).frontendReady) throw new Error("render failure was not persisted");
      await finish(true, "native WebView: populated desktop, recovery after fault, real update IPC failure");
    } catch (error) { await finish(false, String(error)); }
    clearInterval(timer);
  }, 200);
})();
