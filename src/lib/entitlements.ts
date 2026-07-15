import type { CloudUser, Entitlements } from "@/types";

const DEFAULT_REQUESTS_PER_HOUR = 100;

// Personal AGPLv3 build: every capability is unlocked regardless of the
// account plan. The paywall matrix is intentionally bypassed here.
export function getEntitlements(
  _user: CloudUser | null | undefined,
): Entitlements {
  return {
    active: true,
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: true,
    profileLimit: Number.MAX_SAFE_INTEGER,
    requestsPerHour: DEFAULT_REQUESTS_PER_HOUR,
  };
}
