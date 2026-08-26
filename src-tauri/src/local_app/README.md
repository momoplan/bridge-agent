# Local App bounded module

This module is the single desktop-side owner of Local App lifecycle state and runtime
coordination.

- `domain.rs` owns the lifecycle states and the existing serialized snapshot contract.
- `application/` owns in-memory lifecycle transitions and install-task state. It does not
  perform HTTP, process, filesystem, or Tauri I/O.
- `adapters/` owns HTTP health probes, operating-system process supervision, monitoring,
  and Tauri event/channel delivery.
- `mod.rs` is the bounded-module facade used by the desktop composition root.

The serialized lifecycle and status payloads are intentionally unchanged by this refactor.
In particular, the existing numeric `schemaVersion` remains an external compatibility
contract; changing that shape requires a separate protocol decision.
