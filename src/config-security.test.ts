import { describe, expect, it } from "vitest";
import {
  buildConsoleUrl,
  fromUiConfig,
  needsBrowserAuthorization,
  normalizePlatformBaseUrl,
  toUiConfig
} from "./App";

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

  it("requires a fresh browser authorization after relay rejects the saved credential", () => {
    const authorized = toUiConfig(agentConfig("") as never);

    expect(
      needsBrowserAuthorization(authorized, { status: "authorization_required" })
    ).toBe(true);
    expect(needsBrowserAuthorization(authorized, { status: "backoff" })).toBe(false);
  });
});

describe("production platform endpoints", () => {
  it.each([
    "https://baijimu.com",
    "https://www.baijimu.com/",
    "https://baijimu.com/lowcode3",
    "https://www.baijimu.com/lowcode3/",
    "https://api.baijimu.com",
    "https://api.baijimu.com/lowcode3/"
  ])("migrates the legacy SaaS endpoint %s to the canonical API endpoint", (value) => {
    expect(normalizePlatformBaseUrl(value)).toBe("https://api.baijimu.com/lowcode3");
  });

  it("opens the canonical SaaS console independently from the API origin", () => {
    const uiConfig = toUiConfig(agentConfig() as never);

    expect(buildConsoleUrl(uiConfig)).toBe("https://console.baijimu.com");
  });

  it("keeps private deployment console navigation on the private origin", () => {
    const uiConfig = toUiConfig(agentConfig() as never);
    uiConfig.platform.base_url = "https://customer.example.com/lowcode3";

    expect(buildConsoleUrl(uiConfig)).toBe("https://customer.example.com/manager");
  });
});
