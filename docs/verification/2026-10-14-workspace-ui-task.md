# Workspace UI integration task — October 14, 2026

- Problem: integrate the owner-reviewed isolated UI into the desktop with real native operations.
- Branch: `feat/workspace-ui`; base `5f7b048446692e7d3021eeed1302e1d88f188213`.
- Acceptance: second manual launch restores the existing main window without a secondary GTK/webview; Steam background duplicates remain quiet. Capture readiness and saving agree on Overview/Replay. Clip filters, safe filename editing, persisted favorites, folder/device selection, disk capacity, explicit comparison, and collapsible flat navigation work with native DTOs.
- Defaults: new Replay preferences use Alt+Shift+Z for the menu and no direct-save bindings; existing preferences and cleared bindings are preserved.
- Ownership: native activation before GTK; shared service/store own file identities and metadata; audio library owns bounded enumeration; webview owns reversible navigation and drafts.
- Failures: collisions, replacement/symlink targets, persistence failures, duplicate starts/actions, crash recovery, active export/rename conflicts, missing microphones/capacity, cancellation, legacy owner without activation support.
- Checks: focused regression fixtures, `tools/validate.sh full`, separate final lifecycle/persistence review, production frontend visual inspection and WebKit fixtures; affected packaging checks.
- Ordinary verification uses private state/fake hardware; no install, live game, external service or publication is authorized. Native compositor focus is best effort.
- Excluded: session report export and playback-only volume controls, following owner decisions.
- Acceptance follow-up: replace the always-idle Steam banner with first-game guidance only for a successfully loaded empty Library; retain per-game Steam setup and distinguish loading/read failures from an empty catalog.
