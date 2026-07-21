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

clientInfo("bootstrap diagnostics ready");
