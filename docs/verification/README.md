# Verification evidence

- [October 15 workspace refinements](2026-10-15-workspace-refinements.md):
  persistent clip trash, named exports, draft retention, audio status, updater
  restart guards and integrated keyboard reconnect checks.

- [October 14 keyboard reconnect recovery](2026-10-14-keyboard-reconnect.md):
  automatic same-user rediscovery, stale-input/menu cleanup, private fixtures,
  production UI inspection and a local Arch test package.

- [October 13 Steam discovery review and Rust 1.98 validation](2026-10-13-library-discovery.md):
  reviewed app-ID matching and import feedback, isolated production UI checks,
  two behavior-preserving Clippy repairs and full offline validation.

- [October 10 Steam X11 launch integration](2026-10-10-steam-x11.md):
  contributor update, selective preload cleanup, real-wrapper fixtures and
  offline compatibility/package checks; installed-game acceptance remains separate.

- [October 9 Replay save layout and NVIDIA extension discovery](2026-10-09-replay-save-nvidia.md):
  stable duration controls, pending/error status and bounded complete Vulkan
  extension queries; successful NVIDIA encoding still needs hardware acceptance.

- [October 9 temporary tray and live-game window close](2026-10-09-steam-temporary-tray.md):
  revised startup and close policy, compositor-focus regression, current private
  fixtures and production build; installed-game acceptance remains separate.

- [October 9 initial Steam automatic exit](2026-10-09-steam-auto-exit.md):
  historical first implementation and installation, superseded after owner testing.

- [October 9 Replay menu pointer lifecycle](2026-10-09-replay-menu.md): removed
  mouse handles, stale input and view-only polling fixes; local packages built,
  owner-confirmed installed-game menu check on CachyOS.

- [October 8 ultrawide Replay and Intel temperature](2026-10-08-ultrawide.md):
  shared pixel-budget admission, Variable ceilings, package sensor discovery and
  current private-fixture/build evidence; hardware encoding remains unverified.

Add dated reports tied to an exact commit or source-manifest hash. Record commands,
environment, success/failure, excluded scope, and artifact hashes where applicable.
Keep private diagnostics and recordings outside Git and reference them cautiously.
Generated logs live in the ignored `.redunar-build/reports/` directory. A committed
summary should describe the evidence and limitations without dumping private logs.
Historical results in TESTING remain available; new long-form reports belong here.

- [October 8 X11 GLX lookup forwarding](2026-10-08-x11-glx.md): actual-sidecar
  fake-provider repair; reported ARC Raiders cause remains unconfirmed.

- [October 8 native tray task](2026-10-08-native-tray-task.md): direct
  StatusNotifier/DBusMenu client and combined release scope.

- [October 9 native tray and combined candidate](2026-10-09-native-tray.md):
  direct tray lifecycle/protocol tests, local DMS registration and combined checks.
