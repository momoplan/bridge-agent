import type { AppUpdateStatus } from "./types";

// Relay's terminal rejection remains authoritative for this running version.
// Later checks can refresh download information without reopening business UI.
export function reconcileAppUpdateStatus(previous: AppUpdateStatus | null, next: AppUpdateStatus): AppUpdateStatus {
  if (previous?.forceUpdateRequired && previous.currentVersion === next.currentVersion) {
    const decision = next.forceUpdateRequired ? next : previous;
    return {
      ...next,
      updateAvailable: true,
      forceUpdateRequired: true,
      minimumSupportedVersion: decision.minimumSupportedVersion ?? previous.minimumSupportedVersion,
      forceUpdateMessage: decision.forceUpdateMessage ?? previous.forceUpdateMessage
    };
  }
  return next;
}
