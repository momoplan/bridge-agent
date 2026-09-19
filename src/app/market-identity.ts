export interface SourceVersion {
  application: { environmentKey: string; appId: string };
  version: string;
}
export type InstallSource =
  | { kind: "environment"; source: SourceVersion }
  | { kind: "market"; marketKey: string; listingId: string; version: string; source: SourceVersion };

export function marketSelectionKey(app: { installSource?: InstallSource | null }): string {
  const source = app.installSource;
  if (source?.kind !== "market") return "";
  return JSON.stringify([source.marketKey, source.listingId]);
}

export function sameSourceApplication(left?: InstallSource | null, right?: InstallSource | null): boolean {
  return left != null && right != null
    && left.source.application.environmentKey === right.source.application.environmentKey
    && left.source.application.appId === right.source.application.appId;
}
