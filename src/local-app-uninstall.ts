export function isConnectorUninstallStopError(error: unknown): boolean {
  if (!error || typeof error !== "object") {
    return false;
  }
  const candidate = error as { code?: unknown; message?: unknown };
  return (
    candidate.code === "connector_uninstall_stop_failed" &&
    typeof candidate.message === "string"
  );
}
