import type { MarketConnector } from "./types/apps";

/** Summarize exact-version declarations; never accept an author's capability slogan. */
export function marketCapabilitySummary(app: Pick<MarketConnector, "methods" | "events" | "permissions">): string {
  return `${app.methods.length} 个方法 · ${app.events.length} 个事件 · ${app.permissions.length} 项权限`;
}

export function matchesMarketAppQuery(
  app: Pick<MarketConnector, "appId" | "name" | "description" | "version" | "methods" | "events" | "permissions">,
  input: string,
): boolean {
  const query = input.trim().toLocaleLowerCase();
  const fields = [
    app.appId, app.name, app.description, app.version,
    ...app.methods.flatMap((method) => [method.name, method.description]),
    ...app.events.flatMap((event) => [event.name, event.description]),
    ...app.permissions.flatMap((permission) => [permission.id, permission.title, permission.description]),
  ];
  return fields.some((field) => field.toLocaleLowerCase().includes(query));
}
