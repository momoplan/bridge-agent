import type { MarketConnector } from "./types/apps";

/** Summarize exact-version declarations; never accept an author's capability slogan. */
export function marketCapabilitySummary(app: Pick<MarketConnector, "methods" | "events" | "permissions">): string {
  return `${app.methods.length} 个方法 · ${app.events.length} 个事件 · ${app.permissions.length} 项权限`;
}
