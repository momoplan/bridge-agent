export interface LocalAppInstallDeepLinkIntent {
  kind: "local_app_install";
  appId: string;
  shareId: string | null;
}

export interface AppOpenDeepLinkIntent {
  kind: "app_open";
}

export type BaijimuDeepLinkIntent =
  | LocalAppInstallDeepLinkIntent
  | AppOpenDeepLinkIntent;

const SHARE_ID_PATTERN = /^[A-Za-z0-9._~-]{1,256}$/;
const APP_ID_PATTERN = /^[A-Za-z0-9._~-]{1,128}$/;

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
    url.hostname === "open" &&
    url.pathname === "" &&
    Array.from(url.searchParams.keys()).length === 0
  ) {
    return { kind: "app_open" };
  }

  if (url.hostname !== "apps" || url.pathname !== "/install") {
    return null;
  }

  const allowedQueryKeys = new Set(["appId", "shareId"]);
  if (Array.from(url.searchParams.keys()).some((key) => !allowedQueryKeys.has(key))) {
    return null;
  }

  const appIds = url.searchParams.getAll("appId");
  if (appIds.length !== 1 || !APP_ID_PATTERN.test(appIds[0])) {
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

  return { kind: "local_app_install", appId: appIds[0], shareId };
}
