export const CODEX_MARKET_APP_ID = "codex";
export const CODEX_CONNECTOR_ID = "com.baijimu.connector.codex";

export interface CodexInstallDeepLinkIntent {
  kind: "codex_install";
  shareId: string | null;
}

export interface DeviceAuthorizationCompleteDeepLinkIntent {
  kind: "device_authorization_complete";
}

export type BaijimuDeepLinkIntent =
  | CodexInstallDeepLinkIntent
  | DeviceAuthorizationCompleteDeepLinkIntent;

const SHARE_ID_PATTERN = /^[A-Za-z0-9._~-]{1,256}$/;

export function parseBaijimuDeepLink(rawUrl: string): BaijimuDeepLinkIntent | null {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return null;
  }

  if (url.protocol !== "baijimu:" || url.username || url.password || url.port || url.hash) {
    return null;
  }

  if (
    url.hostname === "bridge-agent" &&
    url.pathname === "/device-authorization-complete" &&
    Array.from(url.searchParams.keys()).length === 0
  ) {
    return { kind: "device_authorization_complete" };
  }

  if (url.hostname !== "codex" || url.pathname !== "/install") {
    return null;
  }

  const allowedQueryKeys = new Set(["shareId"]);
  if (Array.from(url.searchParams.keys()).some((key) => !allowedQueryKeys.has(key))) {
    return null;
  }

  const shareIds = url.searchParams.getAll("shareId");
  if (shareIds.length > 1) {
    return null;
  }
  const shareId = shareIds[0] ?? null;
  if (shareId != null && !SHARE_ID_PATTERN.test(shareId)) {
    return null;
  }

  return { kind: "codex_install", shareId };
}
