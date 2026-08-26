export type StartupUpdateGateState = "checking" | "ready" | "update_required";

interface StartupUpdateDecisionInput {
  forceUpdateRequired: boolean;
}

interface StartupComponentHealthInput {
  id: string;
  status: string;
}

export function resolveStartupUpdateGate(
  update: StartupUpdateDecisionInput | null
): StartupUpdateGateState {
  return update?.forceUpdateRequired === true ? "update_required" : "ready";
}

export function configurationStartupIsAllowed(
  gate: StartupUpdateGateState,
  components: StartupComponentHealthInput[] | null | undefined
): boolean {
  if (gate !== "ready") {
    return false;
  }
  const migration = components?.find((component) => component.id === "config_migration");
  return migration?.status === "ready" || migration?.status === "degraded";
}
