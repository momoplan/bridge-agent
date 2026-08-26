import { describe, expect, it } from "vitest";
import {
  configurationStartupIsAllowed,
  resolveStartupUpdateGate
} from "./startup-update-gate";

describe("startup update gate", () => {
  it("blocks configuration and business startup when an update is required", () => {
    const gate = resolveStartupUpdateGate({ forceUpdateRequired: true });

    expect(gate).toBe("update_required");
    expect(
      configurationStartupIsAllowed(gate, [{ id: "config_migration", status: "ready" }])
    ).toBe(false);
  });

  it("does not load configuration until the Rust startup migration reaches a terminal state", () => {
    const gate = resolveStartupUpdateGate({ forceUpdateRequired: false });

    expect(configurationStartupIsAllowed(gate, [{ id: "updater", status: "ready" }])).toBe(false);
    expect(
      configurationStartupIsAllowed(gate, [
        { id: "updater", status: "ready" },
        { id: "config_migration", status: "ready" }
      ])
    ).toBe(true);
  });

  it("allows offline startup only after the migration gate has completed", () => {
    const gate = resolveStartupUpdateGate(null);

    expect(gate).toBe("ready");
    expect(
      configurationStartupIsAllowed(gate, [
        { id: "updater", status: "degraded" },
        { id: "config_migration", status: "degraded" }
      ])
    ).toBe(true);
  });

  it("keeps configuration blocked when the Rust gate skipped migration for an update", () => {
    expect(
      configurationStartupIsAllowed("ready", [
        { id: "updater", status: "degraded" },
        { id: "config_migration", status: "skipped" }
      ])
    ).toBe(false);
  });
});
