import { startRecoveryShell } from "./recovery-shell";

const recovery = startRecoveryShell(document);
// Import/asset failures and the React boundary own fatal startup failures.
void import("./business-entry")
  .then(({ mountBusiness }) => mountBusiness(document.getElementById("root")!, recovery.ready, recovery.fail))
  .catch(recovery.fail);
