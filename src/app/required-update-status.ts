import type { AppUpdateStatus } from "./types";

// Relay's terminal rejection remains authoritative for this running version.
// Later checks can refresh download information without reopening business UI.
export function reconcileAppUpdateStatus(previous: AppUpdateStatus | null, next: AppUpdateStatus): AppUpdateStatus {
  if (previous?.forceUpdateRequired && previous.currentVersion === next.currentVersion) {
    return {
      ...next,
      updateAvailable: true,
      forceUpdateRequired: true,
      minimumSupportedVersion: next.minimumSupportedVersion ?? previous.minimumSupportedVersion,
      forceUpdateMessage: next.forceUpdateMessage ?? previous.forceUpdateMessage
    };
  }
  return next;
}
