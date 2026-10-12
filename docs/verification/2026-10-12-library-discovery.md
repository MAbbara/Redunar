# Library Steam discovery verification — October 12, 2026

Base: upstream/main `7480d06` (v0.1.21). Branch: `codex/library-steam-discovery`.

The discovery dialog now uses Steam app IDs to identify imported Steam games.
Sharing `/usr/bin/steam` or a launcher executable no longer prevents distinct
Steam games from being selected. Native discovery, import validation, and saved
catalog formats are unchanged. Direct desktop entries still match executable paths.

## Evidence

- The behavioral regression failed before the change: an unimported Steam game
  sharing the saved game's launcher had a disabled checkbox.
- All 79 Node UI tests passed on the rebased worktree.
- `tools/validate.sh quick` passed: syntax, formatting, build isolation fixtures,
  both Cargo workspace checks, frontend build, and UI tests. Report:
  `.redunar-build/reports/20261012T155712Z-quick.vlkXsh.log`.
- `python3 output/tauri-redunar/build-native.py` produced a native release app and
  matching capture libraries, Steam bridge, shortcut helper, and update helper.
  Adjacent component hashes verified and the executable had no missing dynamic
  libraries. App SHA-256:
  `ba69b451ce3a1138147e8f6a1a9abe97bc85f551d28b087934abcf5700f666ed`.
- A focused production WebKit check verified selection and submission of a new
  Steam app sharing the existing game's executable. Its dialog screenshot was
  visually inspected: existing entry disabled, new entry checked, Import enabled.
- The first broad WebKit run stopped at a Replay assertion because the page was
  unexpectedly Library; the cause was not established. A complete rerun passed
  all 108 workspace checks, including the updated shared-launcher discovery and
  import fixture. Evidence: `target/ui-review/workspace-checks.json` and
  `target/ui-review/installed-game-discovery.png`.

## Review and manual test

The final diff keeps the change inside the dialog's duplicate check. Both row
selection and Import availability use the same predicate. The native import
command still rediscovers selected IDs and enforces catalog uniqueness. No
migration, profile inheritance, startup/shutdown, privilege, or cleanup change
is involved. The Node tests cover unchanged/changed/unavailable Steam launchers
and desktop executable matching; WebKit covers the production command payload.

Run `.redunar-build/run-library-test.sh` from this worktree after fully quitting
the installed app. The runner uses the owner's existing local state. Scan and
import the previously blocked games, then verify their Library entries. No
package was installed. Full Rust tests/Clippy, hardware capture, and installed
runtime qualification were not performed; this is a local test build, not a
qualified release. To return to the installed app, quit the test instance and
open the installed version normally.

## Follow-up: import feedback and checkbox controls

The owner's follow-up confirmed that the previously blocked games were selectable,
but importing appeared to do nothing. Read-only host inspection found both the
local test app and `/usr/bin/redunar-tauri` running; the installed app held the
`backend-owner-v1.lock` write lease. The catalog still contained four entries.
The native write guard rejects imports from the secondary, read-only service.
The frontend sent this error to a toast outside the modal top layer, hiding the
reason from the user.

Action failures now appear in a focused, accessible alert inside the open dialog.
Opening a dialog or retrying clears the old error. Import shows a disabled
`Importing…` button during the request, preserves selection on failure, and
prevents duplicate submission. Discovery checkboxes use stable 18px controls
with explicit checked, disabled, hover, and keyboard-focus states. Existing
native ownership and persistence boundaries remain enforced.

- The focused production WebKit failure fixture failed before the change with
  `visible owner error`, then passed all seven discovery checks after the fix.
  These cover empty selection, full-row checkbox interaction, error reset,
  backend rejection, retained selection, pending feedback, repeat submission,
  retry, and the native import payload.
- All 79 Node tests and `tools/validate.sh quick` passed. Quick report:
  `.redunar-build/reports/20261012T163532Z-quick.8Z7alz.log`.
- A broader WebKit run stopped on an unexpected Library navigation during a
  Settings assertion. The suite was rerun with the fixture's GTK window refusing
  mouse input (normal programmatic DOM interactions retained); all 112 checks
  passed. No UI assertion was skipped or weakened.
- The import-error screenshot was inspected: the error is visible above the form,
  existing entries remain disabled, and the selected checkbox renders clearly.
- The native app and matching components were rebuilt successfully. The updated
  executable SHA-256 is
  `a1716818abea2f8e811ebac3edf7d4ae5e1dadf4a3312ca7121751f90939855b`.
  This supersedes the earlier test executable hash above.
- The local, ignored test launcher now refuses to open when a same-user Redunar
  process is already running. Fully quit both old instances, including tray
  processes, before starting the rebuilt test app. Existing read-only instances
  do not acquire write access merely because the primary later exits.

No live user catalog was edited by these checks. The owner still performs the
actual import in the restarted test instance. Full release/hardware qualification
remains outside this local UI fix.
