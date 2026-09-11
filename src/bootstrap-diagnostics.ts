import { clientError, clientInfo } from "./client-logger";

function report(kind: string, value: unknown) {
  const message = value instanceof Error ? `${value.message}\n${value.stack ?? ""}` : String(value);
  clientError(`${kind}: ${message}`);
}

window.addEventListener("error", (event) => {
  report("window.error", event.error ?? event.message);
});

window.addEventListener("unhandledrejection", (event) => {
  report("unhandledrejection", event.reason);
});

document.addEventListener("securitypolicyviolation", (event) => {
  // Keep resource identity without persisting query parameters or fragments.
  const resource = event.blockedURI.split(/[?#]/, 1)[0];
  clientError(`securitypolicyviolation: directive=${event.effectiveDirective} resource=${resource} line=${event.lineNumber}`);
});

clientInfo("bootstrap diagnostics ready");
