export type EmbeddedLocalAppView = "mounted" | "transitioning" | "unavailable" | "uninstalling";

export function embeddedLocalAppView(
  lifecycleState: string,
  uninstalling: boolean,
): EmbeddedLocalAppView {
  if (uninstalling) return "uninstalling";
  if (lifecycleState === "running") return "mounted";
  if (lifecycleState === "starting" || lifecycleState === "stopping") return "transitioning";
  return "unavailable";
}
