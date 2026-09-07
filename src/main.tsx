import { startRecoveryShell } from "./recovery-shell";

const recovery = startRecoveryShell(document);
// The independent shell also catches module loading, syntax and asset failures.
void import("./business-entry")
  .then(({ mountBusiness }) => mountBusiness(document.getElementById("root")!, recovery.ready, recovery.fail))
  .catch(recovery.fail);
