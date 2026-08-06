import { describe, expect, it } from "vitest";
import { isConnectorUninstallStopError } from "./local-app-uninstall";

describe("connector uninstall error classification", () => {
  it("only offers force uninstall for a classified stop failure", () => {
    expect(
      isConnectorUninstallStopError({
        code: "connector_uninstall_stop_failed",
        message: "stop hook failed"
      })
    ).toBe(true);
    expect(
      isConnectorUninstallStopError({
        code: "connector_uninstall_failed",
        message: "installation directory is locked"
      })
    ).toBe(false);
    expect(isConnectorUninstallStopError("legacy unstructured error")).toBe(false);
  });
});
