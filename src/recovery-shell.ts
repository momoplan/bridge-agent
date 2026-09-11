import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// No business code, React, or business styles may be imported into this shell.
export function startRecoveryShell(document: Document, reload = () => location.reload()) {
  const panel = document.getElementById("desktop-recovery")!;
  const root = document.getElementById("root")!;
  const detail = document.getElementById("recovery-detail")!;
  const status = document.getElementById("recovery-status")!;
  const title = document.getElementById("recovery-title")!;
  let disposed = false;
  let failed = false;
  let mounted = false;
  let installing = false;
  let checking = false;
  const disposers: Array<() => void> = [];
  const message = (error: unknown) => error instanceof Error ? error.message : String(error);

  function fail(error: unknown) {
    if (disposed || failed) return;
    failed = true;
    window.clearTimeout(bootTimeout);
    root.hidden = true;
    panel.hidden = false;
    title.textContent = "界面暂时无法显示";
    detail.textContent = message(error);
    void invoke("report_frontend_failure", { detail: message(error) }).catch(() => {
      status.textContent = "无法连接客户端，请从系统托盘选择“检查更新与修复”，或使用官方安装包修复安装。";
    });
  }

  async function action(command: string) {
    try { await invoke(command); }
    catch (error) { status.textContent = message(error); }
  }
  function bind(id: string, handler: () => void) {
    const button = document.getElementById(id)!;
    button.addEventListener("click", handler);
    disposers.push(() => button.removeEventListener("click", handler));
  }
  bind("recovery-retry", reload);
  bind("recovery-log", () => { void action("open_startup_log"); });
  bind("recovery-native", () => { void action("open_native_recovery"); });
  bind("recovery-check", () => {
    if (checking) return;
    checking = true;
    status.textContent = "正在检查官方更新…";
    void invoke<{ currentVersion: string; latestVersion: string | null; updateAvailable: boolean }>("check_app_update")
      .then((update) => {
        status.textContent = update.updateAvailable
          ? `发现新版本 ${update.latestVersion}，可点击“安装官方更新”。`
          : `当前版本 ${update.currentVersion}，暂无新版本。可以重试界面或查看日志。`;
      }).catch((error) => { status.textContent = message(error); })
      .finally(() => { checking = false; });
  });
  bind("recovery-install", () => {
    if (installing) return;
    installing = true;
    const button = document.getElementById("recovery-install") as HTMLButtonElement;
    button.disabled = true;
    status.textContent = "正在下载并校验官方更新，完成后将重启客户端…";
    void invoke<{ status: string; version: string }>("install_app_update")
      .then((result) => {
        status.textContent = result.status === "up_to_date"
          ? `当前版本 ${result.version}，暂无可安装的更新。` : "更新已安装，即将重启。";
      }).catch((error) => { status.textContent = message(error); })
      .finally(() => { installing = false; button.disabled = false; });
  });
  void listen<{ message: string }>("app-update-progress", (event) => {
    if (installing) status.textContent = event.payload.message;
  }).then((unlisten) => disposed ? unlisten() : disposers.push(unlisten)).catch(() => {});

  // Global errors are recorded by bootstrap-diagnostics. Only a failed business
  // import/render, an absent root, or a stalled mount can take over the window.
  const pulse = () => {
    if (mounted && !failed && !root.firstElementChild) fail("界面内容已意外退出，请重试或安装官方更新。");
    void invoke("frontend_heartbeat").catch(() => {});
  };
  const timer = window.setInterval(pulse, 3000);
  const bootTimeout = window.setTimeout(() => {
    if (!mounted) fail("界面启动超时，请重试或安装官方更新。");
  }, 20000);
  pulse();

  return {
    fail,
    ready() {
      if (failed || disposed) return;
      mounted = true;
      window.clearTimeout(bootTimeout);
      panel.hidden = true;
      root.hidden = false;
    },
    dispose() {
      disposed = true;
      window.clearInterval(timer);
      window.clearTimeout(bootTimeout);
      disposers.forEach((dispose) => dispose());
    }
  };
}
