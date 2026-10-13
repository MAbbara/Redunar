# Library Steam discovery fix

- Problem: the discovery dialog incorrectly marks every Steam game sharing an imported game's launcher executable as already imported. Missing games cannot be selected for import.
- Acceptance: Steam candidates match catalog entries by Steam app ID; distinct IDs remain selectable even with a shared launcher. Existing Steam entries remain disabled after launcher changes. Direct desktop entries retain executable matching.
- Branch: `codex/library-steam-discovery`, based on upstream/main `7480d06` (v0.1.21), in the dedicated library-steam-discovery worktree.
- Scope/ownership: `output/tauri-redunar/ui/app.js` discovery dialog and behavioral Node regression tests. Native discovery/import and catalog persistence are unchanged.
- State and cleanup: no migration, inherited-profile, cancellation, or cleanup changes. Discovery remains read-only until the owner imports selected entries.
- Checks: reproduce the failing selection before the fix; run all Node UI tests, frontend production build, and the repository quick validation. Build the native app and matching sidecars using the documented host builder.
- Manual acceptance: owner launches the local build, opens Library > Scan installed games, imports previously blocked games, and checks that they appear in the Library. The local runner uses real user state; fully quit the installed app first. No package installation or real-game capture qualification is included.
- Recovery: stop the local app and return to the installed version. The fix changes no persisted formats.

## Follow-up scope

Make import failures visible inside the modal, preserve selected candidates for
retry, expose pending import progress, suppress duplicate submission, and correct
the checkbox appearance using the approved control palette. Verify the native
secondary-instance rejection through an isolated WebKit fixture. Preserve the
backend write-ownership guard; restart the local test instance after quitting
all existing Redunar processes before owner acceptance.
