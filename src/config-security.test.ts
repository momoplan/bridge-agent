import { describe, expect, it } from "vitest";
import { fromUiConfig, needsBrowserAuthorization, toUiConfig } from "./App";

function agentConfig(token = "relay-secret") {
  return {
    platform: { base_url: "https://baijimu.com/lowcode3", workspace_id: 42 },
    upload: { prepare_url: null, inline_limit_bytes: 262144, timeout_secs: 30 },
    relay: {
      url: "wss://relay.baijimu.com/ws/agent",
      agent_id: "device-1",
      token,
      reconnect_secs: 5
    },
    device: { name: "workstation", description: "", tags: [] },
    runtime: {},
    services: [],
    credential_status: { relay_token_configured: true }
  };
}

describe("desktop credential boundary", () => {
  it("never copies a backend relay token into frontend state", () => {
    const uiConfig = toUiConfig(agentConfig() as never);

    expect(uiConfig.relay.token).toBe("");
    expect(uiConfig.credential_status.relay_token_configured).toBe(true);
  });

  it("never sends a relay token back through the frontend save command", () => {
    const uiConfig = toUiConfig(agentConfig() as never);
    uiConfig.relay.token = "unexpected-ui-secret";

    expect(fromUiConfig(uiConfig).relay.token).toBe("");
  });

  it("uses secure credential status rather than a token value for authorization", () => {
    const authorized = toUiConfig(agentConfig("") as never);
    expect(needsBrowserAuthorization(authorized)).toBe(false);

    authorized.credential_status.relay_token_configured = false;
    expect(needsBrowserAuthorization(authorized)).toBe(true);
  });
});
