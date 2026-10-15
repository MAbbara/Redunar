# Keyboard reconnect recovery — October 14, 2026

Local branch `fix/keyboard-reconnect`, based on
`f6dc63f34854b08aa2eb0a660debea868f371785`.
The compatibility build source-manifest SHA-256 is
`317eac0770b23d795737590df88e5950bb5a6055a915e06a3acf1aadcf4c5dd1`.
The manifest predates this evidence summary; implementation files are unchanged.
Checkout-local reports and component digests live in `.redunar-build/reports/`.

## Change and review

The helper previously discovered keyboards only at startup. A keyboard reconnect
could terminate its readers while an unrelated dongle keyboard interface kept
monitoring apparently active. One bounded, nonblocking supervisor now retries
discovery, retires lost/replaced handles by filesystem identity, assigns fresh
input generations and clears stale held keys. Keyboard loss closes an owned menu
through the existing control protocol and releases its mice. Empty readable sets
report Unavailable and recover automatically without preference writes or a
visible webview. See [the behavior contract](../../REPLAY.md#shortcuts-and-menus)
and [ownership/bounds](../../ARCHITECTURE.md#metrics-and-history).

A local lifecycle review traced app startup, saved bindings, native dispatch,
input discovery, access failure/recovery, composite keyboards, menu ownership,
stdin EOF and full-queue shutdown. Existing profile persistence, shortcut wire
configuration, duplicate-edge suppression and view-only pointer behavior remain
compatible. The only status-wire addition is `WAITING KEYBOARD`; matching app and
helper payloads are packaged together. No dependency or persisted format changed.

## Automated evidence

On October 14, CachyOS x86_64 with Rust 1.97.0:

- Full offline validation passed: both Cargo workspaces, Rust tests and strict
  Clippy, production frontend build, Node tests, Python tooling fixtures and
  formatting. Hardware-only tests remained explicitly ignored.
- The focused helper suite passed 30 tests. New cases cover attachment while a
  dongle remains open, same-path replacement, missing/denied/recovered access,
  EOF/read failures, dropped synchronization, handle bounds, stale modifiers,
  duplicate saves, menu close and supervisor join with a full queue.
- The production WebKitGTK fixture passed all 149 checks, including waiting
  feedback and automatic recovery preserving an unsaved shortcut draft without
  saving or Retry activation. Its waiting screenshot was visually inspected.
- The offline Debian 12 compatibility build produced the app, both capture
  libraries, Steam wrapper and both helpers. All six require at most glibc 2.34,
  below the 2.36 ceiling. Its newer compiler reported existing `fetch_update`
  deprecations in replay release bookkeeping and clip metadata.
- The native Arch test package built with no network. All six runtime payload
  hashes match the freshly built components; root ownership and mode 0755 pass.
  Package SHA-256:
  `2611cbc6e79f9aede92f35be997d7fd319846d4aa18a81c3539764775579a366`.

The package is local test material retaining the current 0.1.23-1 metadata,
not a new qualified release. Full multi-format release qualification and
publication remain separate. No package was installed, app restarted, live input
opened for verification, USB device reconnected, or game session controlled.
Live reconnect latency, permissions across seat changes and game/helper CPU costs
remain unqualified. Active indicates a readable keyboard interface and cannot
prove every attached physical keyboard is usable.

New documentation links resolve; the existing historical TESTING link to the
September 23 private local-build status is absent in this isolated checkout and
was not changed.
