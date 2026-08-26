export type StartupUpdateGateState = "checking" | "ready" | "update_required";

interface StartupUpdateDecisionInput {
  forceUpdateRequired: boolean;
}

interface StartupComponentHealthInput {
  id: string;
  status: string;
  detail?: string | null;
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
  return migration?.status === "ready";
}

export function configurationStartupFailureDetail(
  components: StartupComponentHealthInput[] | null | undefined
): string | null {
  const migration = components?.find((component) => component.id === "config_migration");
  if (migration?.status !== "degraded") {
    return null;
  }
  return migration.detail?.trim() || "配置迁移失败，尚未读取业务配置。";
}
