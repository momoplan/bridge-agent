import { describe, expect, it } from "vitest";
import {
  deriveDeviceAuthorizationState,
  deviceAuthorizationLocksCapabilities
} from "./device-authorization-state";

describe("device authorization state", () => {
  it("distinguishes missing authorization from a connection failure", () => {
    expect(deriveDeviceAuthorizationState({
      workspaceId: "",
      relayTokenConfigured: false,
      runtimeStatus: "stopped"
    })).toBe("unauthorized");

    expect(deriveDeviceAuthorizationState({
      workspaceId: "433",
      relayTokenConfigured: true,
      runtimeStatus: "backoff"
    })).toBe("authorized");
  });

  it("treats relay credential rejection as reauthorization, not offline", () => {
    expect(deriveDeviceAuthorizationState({
      workspaceId: "433",
      relayTokenConfigured: true,
      runtimeStatus: "authorization_required"
    })).toBe("reauthorization_required");
  });

  it("uses the local pending session only for the authorizing UI state", () => {
    expect(deriveDeviceAuthorizationState({
      workspaceId: "",
      relayTokenConfigured: false,
      runtimeStatus: "stopped",
      authorizationPending: true
    })).toBe("authorizing");
  });

  it.each(["unauthorized", "authorizing", "reauthorization_required"] as const)(
    "locks platform capabilities while %s",
    (state) => expect(deviceAuthorizationLocksCapabilities(state)).toBe(true)
  );

  it("unlocks capabilities only after authoritative credentials exist", () => {
    expect(deviceAuthorizationLocksCapabilities("authorized")).toBe(false);
  });
});
