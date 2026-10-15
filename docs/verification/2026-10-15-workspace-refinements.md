# Workspace refinements — October 15, 2026

v0.1.24 implementation on `feat/workspace-refinements`, integrating
`fix/keyboard-reconnect` with main at `f6dc63f34854b08aa2eb0a660debea868f371785`.
The full-validation source manifest before adding this evidence summary is
`025bc05a73d1c3fdc239e21b27fe049053386dda2471d723b4c2bdf0ff9c67ce`. The implementation and test files did not change after this pass.
Release qualification records the final committed tree separately.

## Checks and review

- `tools/validate.sh full` passed: 13 tool tests, 103 Node tests, 818 Rust
  test executions across both workspaces, formatting and strict Clippy. Twelve
  hardware-dependent tests remained ignored; no blanket ignored suite was run.
- Production Vite build and isolated WebKitGTK workspace: 149 checks passed.
  Wide, compact and narrow layouts were inspected from generated screenshots.
- Isolated WebKitGTK refinements: 24 checks passed, including trash write failure,
  Cancel, Undo expiry without deletion, Restore, retained drafts across navigation,
  Beta access persistence/failure, restart save failure and native cleanup veto.
- Production Replay WebKitGTK fixture passed with generated private video:
  playback, eight actual frame samples, exact keyboard trim boundaries, filename
  submission, collision retry and configured-folder authority.
- Store/service tests cover persistent bin state across reopening, idempotency,
  favorite/game identity, failed writes, foreign/replaced inodes, bounded named
  exports, collisions, capacity including trashed files and secondary ownership.
- Native tests cover microphone packet freshness and restart admission serialized
  against session launch and export. Restart replaces the process after shutdown;
  close-on-exec descriptors release the old backend lease before new startup.

The final review traced UI commands through the native host, shared service,
store/runtime, persistence and shutdown. Three initial lint findings were repaired;
a budget test fixture was corrected to use valid limits. No validation gate was
weakened. UI fixtures use private native-command responses, not user state.

## Compatibility and remaining acceptance

Trash is a separate inode-bound ledger; media and the existing labels format stay
intact. Older releases display those recordings as ordinary clips. Named exports
use the existing ownership-label format and never overwrite recordings. No
retention timer or permanent-delete control was added. Feature access reuses the
existing Beta preference, defaults to Stable and does not change update sources.

No package was installed and the existing app was not restarted. Existing-window
activation, installed update/restart, actual microphone/keyboard reconnection and
real-game capture still require separately authorized installed-system acceptance.
These fixtures add no hardware/vendor support claim. Release qualification and
package digests remain in the checkout-owned candidate and reports directories.
