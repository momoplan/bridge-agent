# Bundle Connector installation identity

The user approved the complete LOCAL_APP Locator for installation, bound
calls and events. The desktop host uses relay 0.49.0 at the separately
approved Gitee revision 90e93d6f0234f5d22f2b8fd1a86dec468c7d915a
with the existing device-channel feature. The public Connector manifest and
its loopback event handoff keep using the package-local appId.

The device installation record stores the exact Locator and version. The
package, data directory, credentials, capability registry, durable event IDs
and acknowledgments are isolated by Locator. The host verifies the exact
frozen identity and Loopback credentials before using the installation. Two
Connectors managed as separate processes on the same device must declare
distinct local transport ports.

An adoption is a user-initiated install of the same exact version. The source
Owner resolves the already registered legacy source and the host checks the
result against the requested target. The host stops the original installation,
moves its local data and credentials and rekeys the verified queued events.
The original install record is archived after the migration. The Relay
authorization remains tied to the same installed LOCAL_APP, device and
workspaces. Installing an unrelated Connector with the same appId never
adopts the original installation.
