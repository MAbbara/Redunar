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
