import { describe, expect, it } from "vitest";
import {
  configurationStartupFailureDetail,
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

  it("allows offline startup only after a successful migration gate", () => {
    const gate = resolveStartupUpdateGate(null);

    expect(gate).toBe("ready");
    expect(
      configurationStartupIsAllowed(gate, [
        { id: "updater", status: "degraded" },
        { id: "config_migration", status: "ready" }
      ])
    ).toBe(true);
  });

  it("keeps configuration blocked and exposes the migration root cause on failure", () => {
    const components = [
      { id: "updater", status: "ready" },
      {
        id: "config_migration",
        status: "degraded",
        detail: "旧版本配置迁移失败: missing installation record"
      }
    ];

    expect(configurationStartupIsAllowed("ready", components)).toBe(false);
    expect(configurationStartupFailureDetail(components)).toBe(
      "旧版本配置迁移失败: missing installation record"
    );
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
