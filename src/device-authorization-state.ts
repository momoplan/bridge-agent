export type DeviceAuthorizationState =
  | "unauthorized"
  | "authorizing"
  | "reauthorization_required"
  | "authorized";

export interface DeviceAuthorizationInputs {
  workspaceId: string;
  relayTokenConfigured: boolean;
  runtimeStatus?: string | null;
  authorizationPending?: boolean;
}

export function deriveDeviceAuthorizationState({
  workspaceId,
  relayTokenConfigured,
  runtimeStatus,
  authorizationPending = false
}: DeviceAuthorizationInputs): DeviceAuthorizationState {
  if (authorizationPending) {
    return "authorizing";
  }
  if (runtimeStatus === "authorization_required") {
    return "reauthorization_required";
  }
  if (!workspaceId.trim() || !relayTokenConfigured) {
    return "unauthorized";
  }
  return "authorized";
}

export function deviceAuthorizationLocksCapabilities(
  state: DeviceAuthorizationState
): boolean {
  return state !== "authorized";
}
