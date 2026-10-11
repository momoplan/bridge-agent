import { startRecoveryShell } from "./recovery-shell";
import { initializeLayoutCapabilities } from "./layout-capabilities";

const recovery = startRecoveryShell(document);
initializeLayoutCapabilities(document);
// Import/asset failures and the React boundary own fatal startup failures.
void import("./business-entry")
  .then(({ mountBusiness }) => mountBusiness(document.getElementById("root")!, recovery.ready, recovery.fail))
  .catch(recovery.fail);
