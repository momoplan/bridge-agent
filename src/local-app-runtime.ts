export type EmbeddedLocalAppView = "mounted" | "transitioning" | "unavailable" | "uninstalling";

export function embeddedLocalAppView(
  lifecycleState: string,
  uninstalling: boolean,
): EmbeddedLocalAppView {
  if (uninstalling) return "uninstalling";
  if (lifecycleState === "ready") return "mounted";
  if (["installing", "starting", "stopping", "upgrading", "recovering"].includes(lifecycleState)) {
    return "transitioning";
  }
  if (lifecycleState === "uninstalling") return "uninstalling";
  return "unavailable";
}
