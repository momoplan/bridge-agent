# Device personal image ownership

The user directed screenshots to the device-authorizing user's personal folder and explicitly instructed:
“修复一下，不改project service” (2026-10-11). The owner is Workspace File Service. Project Service is unchanged.

Before: Bridge screenshot -> Relay upload preparation -> Project OSS signing; Relay sends bare user IDs,
no personal file reservation is completed, and an OSS object key masquerades as fileId.
After: Bridge -> existing Partner authentication -> Workspace File Service personal prepare/complete; bytes go directly
through its signed PUT ticket. Relay remains the command/result transport. No new package dependency, identity issuer,
secret, database, stored session or cross-user file grant is introduced. The user's existing device PAT binding is reused.

Alternatives rejected: adding a Project signing API duplicates file ownership; trusting bare actor headers bypasses
identity verification; anonymous image URLs expose private files. The personal file API remains the sole storage and
permission owner. Uploaded file IDs are stable; temporary staging URLs never enter tool results.

Workspace File Service must deploy its additive public upload routes before this client is installed. Client consumers
must recognize `personal_file` and retrieve content with the owner's authorization. Existing Connector v1 upload and
its separately advertised host capability are outside the screenshot repair and are not changed. Generic remote invoke
envelopes, method inputs, inline screenshots and Relay control transport remain unchanged.

No file or database migration is required. Client rollback restores the previous (known broken large-image) behavior,
not a successful alternative upload path. Files already committed remain managed by Workspace File Service.

On 2026-10-11 the current user explicitly confirmed both exact MINOR releases: platform Component
workspace-file-service 7.0.1 -> 7.1.0 and independent desktop project bridge-agent 0.10.3 -> 0.11.0.
The screenshot repair is the approved scope; Connector upload remains unchanged.

Tests cover owner selection, environment binding, failure propagation and the prepare/PUT/complete ordering. Production
acceptance must capture on the authorized device, read the committed file by ID using its owner credential, and verify
that another user's credential cannot read it. Do not report successful production upload before this acceptance.
