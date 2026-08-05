export const CODEX_MARKET_APP_ID = "codex";
export const CODEX_CONNECTOR_ID = "com.baijimu.connector.codex";

export interface CodexInstallDeepLinkIntent {
  kind: "codex_install";
  shareId: string | null;
}

export type BaijimuDeepLinkIntent = CodexInstallDeepLinkIntent;

const SHARE_ID_PATTERN = /^[A-Za-z0-9._~-]{1,256}$/;

export function parseBaijimuDeepLink(rawUrl: string): BaijimuDeepLinkIntent | null {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return null;
  }

  if (
    url.protocol !== "baijimu:" ||
    url.hostname !== "codex" ||
    url.pathname !== "/install" ||
    url.username ||
    url.password ||
    url.port ||
    url.hash
  ) {
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
