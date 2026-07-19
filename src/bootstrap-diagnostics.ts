import { invoke } from "@tauri-apps/api/core";

function report(kind: string, value: unknown) {
  const message = value instanceof Error ? `${value.message}\n${value.stack ?? ""}` : String(value);
  void invoke("report_frontend_bootstrap_event", { message: `${kind}: ${message}` }).catch(() => {});
}

window.addEventListener("error", (event) => {
  report("window.error", event.error ?? event.message);
});

window.addEventListener("unhandledrejection", (event) => {
  report("unhandledrejection", event.reason);
});

void invoke("report_frontend_bootstrap_event", { message: "bootstrap diagnostics ready" }).catch(() => {});
