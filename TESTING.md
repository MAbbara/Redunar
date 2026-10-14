# Verification guide

Current Tauri and shared-backend checks, reviewed September 26, 2026. Run commands
from the repository root unless explicitly stated otherwise. Use existing offline
dependencies. Root Cargo commands do **not** include the separate Tauri workspace.

## Ultrawide Replay and CPU sensors

Ordinary fixtures also cover the shared pixel budget through Vulkan/OpenGL
source selection, protocol exported/copied messages, daemon DMA-BUF/stream
validation and encoder requests at 5120×1440. They exercise Variable FPS's
72 FPS encoder ceiling, reject oversized/over-budget and short backing buffers,
and retain existing failed-ownership/resize cleanup tests. Intel sensor fixtures
cover package labels, unrelated/core-only sensors, cached paths, missing and
invalid readings. `tools/validate.sh full` runs these without GPU access.
The production WebKit workspace fixture includes the resolution/rate rejection
copy. Hardware save/decode, driver coded-extent limits and sustained performance
at these source shapes require separate acceptance.

See the [October 8 verification](docs/verification/2026-10-08-ultrawide.md) for
the current build, checks and remaining hardware acceptance.

## Replay save layout and extension discovery

Fake Vulkan driver fixtures cover complete lists above the old 256-entry encoder
limit, count/fill growth (`VK_INCOMPLETE`), shrinkage, empty lists, oversized or
persistently unstable lists, invalid written counts and preserved driver errors.
Both capture export and encoder discovery use the shared helper. Native/Node
fixtures distinguish pending frames from explicit failures and source rejection.
The production WebKit fixture compares save-control positions across all eight
durations at wide and compact widths, including assigned/unassigned and cleared
shortcuts; saves remain disabled for pending or failed recording.

See the [October 9 follow-up](docs/verification/2026-10-09-replay-save-nvidia.md)
for dated checks and remaining NVIDIA hardware acceptance.

## Workspace UI and existing-window activation

The October 14 integration is based on `5f7b048` on `feat/workspace-ui`.
Its [task and acceptance scope](docs/verification/2026-10-14-workspace-ui-task.md)
cover the reviewed workspace design and native operations. Run the documented
full gate before merge, followed by the production WebKit fixtures:

```sh
xvfb-run -a python3 output/tauri-redunar/tests/workspace-webview.py
xvfb-run -a python3 output/tauri-redunar/tests/replay-webview.py
```

These serve the built production frontend with private native-command fixtures.
They cover shared readiness/save controls and saved shortcuts, stable positions
at wide/compact widths, filename editing/collision recovery, duplicate favorites,
filters without replacing playback, actual-capacity DTOs/low-space guidance,
missing microphone selection and retry, folder cancellation/default reset,
explicit same-game comparison and collapsible navigation. First-game guidance
is checked against pending, failed, empty, recovered, and populated catalog reads,
including its Library action and narrow layout. Screenshots and
assertion reports belong to `.redunar-build/reports/workspace-ui` and `replay-ui`.

Store/service fixtures cover filename ownership, media/game/favorite preservation
across restart, crash-interrupted rename, collision/replacement/symlink rejection,
metadata persistence failure and read-only ownership. Native export tests cover
mutation rejection while an export owns its source and admission recovery after
failure. Audio fixtures bound input enumeration and exclude monitor/output nodes.
Fresh preferences assign only Alt+Shift+Z; legacy and cleared bindings retain
compatibility coverage.

The private desktop runner below also starts four concurrent secondary activation
clients against the actual Tauri window with tray on/off and manual ownership.
It asserts a single visible webview, unchanged preferences and retained ownership.
Activation unit fixtures cover pending startup, repeated requests, timeout,
malformed/slow clients, stale sockets, unsafe targets and cleanup/restart.
Xvfb does not qualify compositor focus, an installed upgrade, hardware audio or
live-game capture. Those remain separately authorized acceptance. Current run
results and exact source manifests are kept under `.redunar-build/reports`;
production build source/binary manifests belong to `.redunar-build/linux`.

## Replay menu pointer fixtures

The shortcut helper's fake mouse fixtures cover queued gameplay clicks/motion
at menu opening and reopening, fresh input after queue cleanup, failed grabs,
failed reads and bounded cleanup under continuous input. Every acquisition or
cleanup failure releases all mice. Private socket pairs verify that view-only
mode sends heartbeats without reading unowned pointer input, while owned clicks
still reach the menu and read failures request normal cleanup. Existing Escape,
duplicate keyboard edge, inactivity and no-mouse fallback checks remain active.
The reported Alt+Shift+Z chord also runs through the real keyboard monitor with
composite duplicate edges and modifier releases; it refreshes mouse discovery
only when opening, opens once, remains open through a heartbeat, and closes on
the next complete chord. Device discovery is replaced with an empty fixture,
so this monitor test never opens real input devices.

```sh
cargo test --locked --offline -p redunar-hotkeys --target-dir .redunar-build/checks/root
```

These tests use no input devices or game/GPU. The owner confirmed the installed
test build's menu works on October 9; this is separate from automated fixtures
and does not qualify broader Replay or other hardware.
See the [October 9 verification](docs/verification/2026-10-09-replay-menu.md) for
the exact validated source and local test package hashes.

## Native tray fixtures

The Tauri test suite exports real StatusNotifierItem and DBusMenu objects on
private D-Bus instances with fake watchers and hosts. It covers registration,
menu actions, host/owner/bus loss, delayed replies, repeated connections and
failed startup without changing the desktop tray or app preferences.

Cleanup checks require the connection's transport to be closed immediately after
tray shutdown. The existing three-second fixture deadline then requires GIO's
closed notification and the bus's removal of that unique name. Disconnect
notifications and requests from a different connection run on separate workers;
the first NameHasOwner reply does not establish failed cleanup. A socket left
open or a name retained beyond the deadline still fails acceptance.

## Steam Play entry point

The packaged-wrapper preload fixtures use private one-shot brokers and fake
commands to check native Steam Runtime/pressure-vessel launches, direct and
nested Proton paths with non-UTF-8 bytes, inherited Redunar copies, mixed
space/colon preload lists, unrelated libraries, preserved Vulkan/session fields
and fail-open activation. Fixture library paths do not exist; no game, GPU or
graphics library is opened. Full validation includes these tests:

```sh
cargo test --locked --offline -p redunar-platform --bin redunar-steam-launch --test steam_preload --test steam_play --target-dir .redunar-build/checks/root
```

The [October 10 task](docs/verification/2026-10-10-steam-x11-task.md) records
the PR integration and the boundary of the friend's reported X11 check.

The [October 5 combined live check](docs/verification/2026-10-05-combined-live-check.md)
records the Vulkan symbol-preemption regression, corrected bounded fixture,
local/unpacked shortcut-helper lookup, package checks and current AMD evidence.

Ordinary private fixtures cover the real `redunar-steam-launch` binary requesting
preparation before exec, fail-open literal argv, background startup with a fake
app/user-manager client, sanitized literal desktop/audio environment, unavailable
or timed-out manager cleanup, same-user PID credentials, exact imported identity, busy-session rejection,
PID reuse/disappearance, End retaining a live game's lock, and one history record.
Run `tools/validate.sh full` for both Cargo workspaces and the production UI build.
The WebKit workspace fixture checks the always-visible per-game Steam setup
panel, exact option copying, both Library tabs, direct-game omission, configured
confirmation only from native evidence, other ambiguous reasons,
unavailable/error retries, selection preservation and compact layout. It captures
`steam-play-setup.png`, `steam-play-configured.png`, and `steam-play-compact.png`.
The focused `steam-setup.test.mjs` suite covers detached panels and overlapping
checks without touching a real Steam configuration. See the original
[Steam Play task brief](docs/verification/2026-10-05-steam-play-task.md) and the
[October 6 setup discovery evidence](docs/verification/2026-10-06-steam-setup.md).
The [wording follow-up](docs/verification/2026-10-06-steam-setup-copy.md) records
the typed running-Steam state and plain launch/test guidance, preserving other
configuration errors and the unconfirmed gate.
The [Compact follow-up](docs/verification/2026-10-06-compact-steam-setup.md)
records the simplified panel, conditional retry and focus/draft preservation.
The [background lifetime evidence](docs/verification/2026-10-06-steam-background-lifetime.md)
addresses the October 6 owner-tested ARC Raiders save/playback followed by a
stuck exit: the game/Proton exited, but Steam's reaper still owned Redunar.

Live acceptance remains separate: with a matching installed app/wrapper/libraries,
import a native Steam game and save its displayed app-specific Launch Options.
Quit Redunar, click Play in Steam, confirm the background owner/tray (or visible
fallback), profile, metrics, Replay readiness, save/playback and natural cleanup.
After natural game exit, verify Steam returns to Play, Redunar releases its game
session/launch lock and one history entry appears. With Close to tray disabled,
automatic startup must keep the main window hidden and create a temporary tray
icon without changing the preference; game exit must quit that automatic owner
and remove the icon after successful cleanup. Open Redunar from the temporary
tray: the window must appear, the icon must disappear and the app must remain
running after game exit. Manually started Redunar with Close to tray disabled
must create no icon.
While a game is live, open Redunar, then close its window with Close to tray
disabled. The app must return to a temporary tray entry without ending Replay
or the session, then quit/remove the icon after natural game exit and cleanup.
Repeat after reopening: the temporary icon must disappear and the app stay
running; closing again must restore exit after the game. Repeat with Redunar
started manually before Steam. Idle close with the setting off must still quit.
If tray registration is unavailable, live-game close must keep the window
reachable and show an error. Explicit tray Quit retains its existing End/cleanup
path and must not kill the game.
Repeat after restoring and clicking or typing in the window: Redunar must remain
running. Automatic compositor focus at startup or after game exit must not
prevent exit when the window was otherwise unused.
Enabled Close to tray keeps the hidden/tray owner available after game exit;
a missing tray host uses the unfocused taskbar fallback. Verify Redunar's parent
is the user manager rather than the game
reaper. Do not mistake process-group separation for independent ownership.
Repeat with Redunar already open and with an unsaved-in-Steam option edit; a
second active game must continue without taking the first session. Test End and
Quit without killing the game. Flatpak Steam and NVIDIA success remain outside
ordinary fixture evidence. Do not run these hardware/installation checks without
authorization.

Native lifecycle fixtures cover completion before GTK attachment, callback
dispatch without webview polling or ownership locks, stale/duplicate completion,
new-launch/shutdown arbitration, End while the game remains live, current tray
preferences, corrupt preference reads, failed history recording and explicit
retry. These use fake snapshots and test-owned processes/state. The isolated
startup probe exercises the production Tauri/GTK calls with no desktop tray host:

```sh
cargo build --locked --offline --manifest-path output/tauri-redunar/src-tauri/Cargo.toml --example steam_background_probe --target-dir .redunar-build/checks/tauri
python3 tools/check-steam-background.py
```

The runner uses a private bus without host service activation, private runtime
and temporary directories, and allocates displays sequentially. Fake-host cases
check hidden startup with either preference, temporary-icon removal on Open,
unchanged preferences and retention. Missing-host cases check fallback reachability,
no startup focus request/actual focus and removal of a disabled preference's
temporary icon after the window becomes reachable. Additional cases force native focus without input, then
send real X11 key/button events through GTK to the webview using the fixture-only
system XTest library: focus alone must leave exit
eligible, while input must retain the app. Passive focus/map/pointer events are
covered by the native event classification test. Native parking cases exercise
Close/Open/Close, both preferences, a manual owner, unavailable registration and
canceled pending close with a simulated live-game service callback; no game or
capture process is created. The supervisor fixtures distinguish live ownership
after End from completion with failed history. Xvfb without a window manager
does not establish a compositor's minimize/taskbar behavior; the live matrix above
remains separate acceptance. See the [October 9 task](docs/verification/2026-10-09-steam-auto-exit-task.md)
and [verification evidence](docs/verification/2026-10-09-steam-auto-exit.md).

## GLX loader forwarding fixtures

The focused command exercises the actual Cargo-built preload sidecar with C
fixture providers:

```sh
cargo test --locked --offline -p redunar-capture-opengl --test glx_loader --target-dir .redunar-build/checks/root
```

The fixture needs `cc` and GNU `timeout`, with no display,
Steam, real game or GPU. Cases cover local/global visibility, a different local
provider after the global hook is used, promotion to global visibility, cached
missing forwarding targets, original call counts and loader errors. Each fixture
process has a ten-second deadline. Full validation runs this test automatically.
See the [X11 GLX task](docs/verification/2026-10-08-x11-glx-task.md) and
[October 8 evidence](docs/verification/2026-10-08-x11-glx.md).

## Installer shell and dependency fixtures

`tools/test-installer.sh` uses signed local fixture files and fake package
managers; it performs no installation or network download. It covers the
rendered Debian installer and the Arch dependency path with PipeWire, PulseAudio,
or neither already installed, including CachyOS detection through `ID_LIKE=arch`,
preserving exact package argument boundaries.
Corrupted packages and invalid signatures must fail before installation.

All `tools/validate.sh` modes check the source installer with `sh -n` before
native compilation and also with `dash -n` when available. The release gate
runs the full installer fixture suite. A host whose `sh` is Bash does not
establish POSIX portability; repeat the fixture suite in an isolated Debian or
Ubuntu environment whose `sh` is `dash` before qualifying installer changes.
These fixtures do not establish real distribution installation/runtime support.

## September 26, 2026 v0.1.10 release preparation

Source `fc6acfd` plus the Variable FPS Replay-menu fix and v0.1.10 metadata
passed the release-input check, targeted root and separate Tauri offline Cargo
checks and strict Clippy, formatting, JavaScript syntax, AppStream and desktop
metadata validation, and the locked license-inventory check. Root Clippy
allowed only the two unchanged OpenGL `clippy::ptr_eq` findings. The offline
Debian 12 release build passed the glibc 2.36 ceiling. Local Fedora, Debian,
Arch, openSUSE, and portable packages were built from that payload. The first
host Tauri check was interrupted when the release build cleared their shared
target directory; its sequential rerun passed. No automated tests or v0.1.10
installed-app run were performed. The owner observed the menu fix in ARC
Raiders using the same code in a local 0.1.9 RPM.

The later v0.1.10 audio candidate adds explicit PipeWire sink-monitor capture,
keeps production audio ingestion independent of video encoding and save drain,
snapshots audio after draining video, and realigns PCM timestamps after capture
gaps. On September 26, root daemon offline
Cargo check and strict Clippy, formatting, diff whitespace validation, and the
offline Debian 12 release build passed; the build report capped required glibc
at 2.34. The first live ARC Raiders clip after the PipeWire monitor change had
audible Opus audio but only 8.57 seconds of audio for 14.25 seconds of video,
which prompted the independent audio-buffer change. A second clip after that
change still had only 11.32 seconds of audio for 28.36 seconds of video. The
PipeWire helper's PCM output was then 17.5 seconds behind its elapsed runtime,
consistent with the missing tail; a subsequent 20-second sample ran at real
time while retaining that earlier deficit. The audio timestamp realignment
and MP4 gap preservation passed root offline Cargo check, strict Clippy,
formatting, diff whitespace validation, and a second offline Debian 12 release
build on September 26. A later ARC Raiders clip from that build had 299.88
seconds of video and 298.61 seconds of Opus packets, but the owner identified
the audible content as browser audio rather than game audio. A short PipeWire
sink-monitor probe captured non-silent output and numeric PipeWire links showed
ARC Raiders streams feeding the same sink. Short direct probes of those game
streams were nearly silent while the game was likely backgrounded. Astra's
independent review found no confirmed capture-route bug; a simultaneous
foreground/background level comparison is still needed. Game-audio capture
is not yet claimed fixed. A later foreground-only PipeWire probe showed ARC
Raiders and the output monitor at matching non-silent levels while browser
audio was paused. The next 13.99-second clip contained 7.98 seconds of Opus
packets across a 1.92-second gap, ending 4.08 seconds before video. The owner
heard game sound in parts but said it belonged to earlier gameplay. Astra
reviewed the spool and MP4 origins and found no multi-second offset there;
it identified synchronous route discovery in the PCM-reading thread as a
plausible source of queued, stale audio. The next candidate moves route
discovery to a separate worker, bounds each command to one second, anchors
audio startup to the current monotonic clock, and reconnects rather than
timestamping stale PCM as live. Live A/V synchronization remains to be checked.
The corrected candidate passed root offline daemon/audio Cargo check and strict
Clippy, formatting and diff whitespace checks, and the Debian 12 offline release
build on September 26. In the local ARC Raiders session from that build, the
owner reported that the newly saved clip's sound worked. FFprobe showed
1,147 video packets through 14.099 seconds and 705 audio packets from 0.007
through 14.107 seconds, without a gap greater than 100 ms in either track.
The log also showed that repeated slow route checks could temporarily omit the
PipeWire candidate and trigger an unnecessary reconnect. A follow-up change
ignores a missing backend in one check and waits for either a different sink
on the same backend or an actual capture stall/error. That follow-up passed
root strict Clippy. On September 26, the full `tools/check-tauri-release.sh`
gate passed with the follow-up: Debian 12 compatibility build, root and Tauri
checks, 87 Tauri Rust tests passed with two ignored, 9 and 20 tests in the
other Rust targets, 57 UI tests, license and metadata validation, signed
temporary release assets, and local Fedora, Debian, Arch, openSUSE, and
portable package checks. The follow-up route-omission guard has not separately
been exercised in another live game session.

## September 26, 2026 v0.1.9 release preparation

Source commit `897fdcd` plus the v0.1.9 version and package metadata changes
passed the release-input check, root and separate Tauri offline Cargo checks,
the frontend production build and JavaScript syntax check, AppStream and
desktop metadata validation, the locked license-inventory check, and diff
whitespace validation. The offline Debian 12 release build passed its glibc
2.36 ceiling. Local Fedora, Debian, Arch, openSUSE, and portable packages
were built from that payload. No automated or installed-app runtime test was
run for this release preparation; GitHub's release workflow builds and signs
the published packages.

## September 26, 2026 production Variable FPS integration

The v0.1.8 candidate based on `eff0f9c` plus the production Variable FPS
integration passed offline root and separate Tauri `cargo check --all-targets`,
the frontend production build and JavaScript syntax check, and the offline
Debian 12 release build. Local Debian, Arch, openSUSE, Fedora, and portable
packages were rebuilt from that source. The release build report records a
glibc 2.36 ceiling. This is source and package validation, not an installed
upgrade or another distribution's runtime test.
Separate Tauri strict Clippy passed. Root strict Clippy passed with only
`clippy::ptr_eq` allowed for two unchanged OpenGL interposer comparisons;
without that allowance it stops on those two existing lines.

On AMD/RADV in the local Wayland session, the production Vulkan probe with
Variable FPS saved a 640×360 MKV through the real Replay menu. The saved video
had 339 packets averaging 167.5 FPS, mostly 5–6 ms apart; the fixture reported
about 170 game FPS. The same production probe with a GLX game through Xwayland
saved 238 packets averaging 116.7 FPS, mostly 8–9 ms apart; the fixture
reported about 118 game FPS. Both reported active metrics overlay, buffering,
and completed save. These observations validate accepted game presents and
their encoded timestamps on this host; they do not prove every present is
captured or that playback tracks compositor scanout exactly.

The isolated fixed-rate Vulkan Tauri session acceptance passed outside the
sandbox. Its first sandbox run stopped at the hardware validation gate before
capture. The Xvfb OpenGL Variable FPS attempt exposed no Replay candidate;
the real Xwayland GLX run above supplied the X11-client evidence instead.
The WebKit workspace fixture reached the Global settings page and produced a
screenshot but stopped on its existing Glacier overlay-accent assertion before
the Replay controls. The changed recording selector therefore has a successful
production build but no completed fixture visual assertion. Native Xorg,
NVIDIA hardware, installed upgrades, and other-distro runtime behavior remain
unverified.

## September 26, 2026 initial v0.1.8 candidate

The initial v0.1.8 source at `eff0f9c` passed the release-input check, offline
root and Tauri Cargo checks, formatting, diff whitespace, AppStream and desktop
metadata validation, and the locked license-inventory check. The production
frontend and native binaries built in the offline Debian 12 image; all six
native components reference at most glibc 2.34, below the 2.36 release ceiling.
Local Fedora, Debian, Arch, openSUSE, and portable packages built from that
payload. The release workflow remains responsible for production signing and
uploading. No automated tests, NVIDIA hardware run, installed upgrade, or
other-distribution runtime run was performed for this preparation.

## September 26, 2026 NVIDIA beta diagnostic review

On source based on v0.1.7, the NVIDIA beta encoder path gained opt-in,
rate-limited stage/reason diagnostics and NVIDIA-only device selection; the
AMD/RADV zero-level workaround was restricted to AMD. Replay audio operational
messages no longer include output-device names or sound-server error text.
Failed producer-release acknowledgements are retained and retried on later
pump turns. Offline root
`cargo check --offline -p redunar-daemon -p redunar-capture-vulkan`, separate
Tauri `cargo check --offline --manifest-path output/tauri-redunar/src-tauri/Cargo.toml`,
strict root and Tauri Clippy, workspace formatting checks, and `git diff --check`
passed. No NVIDIA
hardware, game run, automated tests, or package build was used for this review.

## September 25, 2026 isolated variable-frame-rate media probe

On source `1ae716e` plus the uncommitted
[`tools/probe-vfr-replay.py`](tools/probe-vfr-replay.py), Fedora 44's FFmpeg
8.1.2 generated a three-second H.264 clip from synthetic 156, 120, and 136
FPS inputs without resampling. MP4 and remuxed MKV each decoded 412 frames:
156/120/136 in successive one-second sections. MP4 median frame intervals
were 6.410/8.333/7.353 ms, with no section exceeding 0.001 ms error from
its requested interval. MKV's one-millisecond timestamp scale produced up to
0.667 ms interval error. A copy-only MP4 preparation matching the Tauri
player's video path retained all 412 frames and the MKV timestamps. A
fixed-60-FPS comparison contained 180 frames.

Reproduce without starting Redunar or accessing game hardware:

```sh
python3 tools/probe-vfr-replay.py /tmp/redunar-vfr-156-120-136.mp4
```

This proves that the local media tools can produce and decode a VFR clip at
these rates. It does not exercise Redunar's recorder, hardware encoder,
presentation feedback, display refresh, audio sync, visual playback, WebKit
playback, or sustained game performance. A headless `ffplay` attempt timed out
after a PulseAudio wake error in this sandbox; decoded-frame counts and
timestamps are the media evidence here. Production capture and stream
validation accepted only 30/60/120 FPS at the time of this isolated media
probe. It was not evidence that the production recorder could capture 156 FPS.

## September 25, 2026 v0.1.7 release preparation

The uncommitted v0.1.7 source passed the release-input check, root and Tauri
offline Cargo checks, Tauri strict Clippy, frontend production build, JavaScript
syntax check, Python and shell syntax checks, AppStream validation, license
inventory check, and local Fedora, Debian, Arch, openSUSE, and portable package
builds. The offline Debian 12 release build passed the glibc 2.36 ceiling; its
`redunar-update-helper` SHA-256 is
`59038f2c6edd04635e6db5909e9909747761a9977eca8fad2d500afbd5079486`.
No automated test suites or live polkit upgrade were run for this preparation.
The publication workflow will sign and upload the official release assets.

## September 25, 2026 local polkit updater implementation

The uncommitted source tree built with `output/tauri-redunar/build-native.py`;
the resulting update helper SHA-256 is
`6d41c73975bc8ae1ac33542bb7b55f30531b7837db98975024e75d8a3464cf06`.
The Tauri `cargo check --offline --all-targets`, strict Clippy, frontend
production build, JavaScript syntax check, package staging, and local Fedora,
Debian, and openSUSE package builds passed. The rebuilt Settings page was
visually inspected in a local browser preview, where native controls are
disabled. No automated tests, installed-app upgrade, or live polkit/package
manager transaction were run; Arch packaging and other-distro runtime behavior
remain unverified on this host.

## September 25, 2026 v0.1.6 release preparation

For v0.1.6 release preparation on September 25, source based on `cfa387b`
plus the Settings redesign and version bump passed `npm run build`,
`node --check output/tauri-redunar/ui/app.js`,
`tools/check-release-inputs.sh v0.1.6`, AppStream validation, and the offline
Debian 12/glibc 2.36 release build. The Settings page was visually inspected
in the local browser preview; its controls are disabled there because the
native backend is absent. A local Fedora RPM was built as
`0.1.6-1.local.fc44`, SHA-256
`c0dc0f7b4baf3dce24c858dcbccd7bdce6d8a9178aec408baace70fb5cb3f340`.
No automated tests, installed-app run, or signed publication were performed
for this preparation; the GitHub release workflow remains responsible for
building and signing published packages.

## September 25, 2026 NVIDIA beta implementation check

Build `0.1.4` from `117aef7` plus the uncommitted Beta access and NVIDIA
changes: the root daemon `cargo check --offline`, the separate Tauri
`cargo check --offline`, the frontend `npm run build`, root strict Clippy for
the affected daemon/platform/NVML/Vulkan crates, Tauri strict Clippy,
`cargo fmt --all -- --check`, and `git diff --check` passed. No NVIDIA GPU was
available on this host, so NVML readings, in-game NVIDIA metrics, and NVIDIA
Replay recording remain unverified. The browser's local-file policy blocked
visual inspection of the changed Settings view. No tests or game runs were
performed for this change.

For the v0.1.5 release preparation on September 25, the committed feature source
`16ed7d0` plus the local version bump passed `tools/check-release-inputs.sh
v0.1.5`, both offline Cargo checks, `build-native.py`, package staging, and a
local Fedora RPM build. The host-built RPM requires glibc 2.39 and is **not**
the Debian 12/glibc 2.36 release artifact. That compatibility build and
production signing remain the GitHub release workflow's responsibility.

## Choose checks for the change

| Change | Required evidence |
| --- | --- |
| Documentation only | Source/decision consistency, local links, and command paths; no app rebuild required |
| Frontend logic | Relevant Node tests and production build; affected UI visually inspected |
| Native commands/lifecycle | Focused Tauri Rust tests, build, Clippy, and relevant bridge checks |
| Shared backend/protocol | Relevant root crate tests, formatting/Clippy, and affected native caller tests |
| Capture/overlay runtime | Fixtures first, then explicitly scoped native/GPU/game validation where needed |
| Packaging/install behavior | Full Tauri release/staging checks; installation only in an authorized workflow |

Do not repeat an expensive full gate without new changes or a reason. A bug fix
needs meaningful regression coverage of its failure, not tests that mirror the
implementation. Ordinary tests use isolated state, fake hardware, and synthetic
media. Never run destructive hardware tests automatically.

## September 23–24, 2026 takeover revalidation

The full offline Tauri release gate passed on an isolated snapshot of the
current dirty working tree. Its native artifacts stayed within the Debian
12/glibc 2.36 baseline. Tauri tests passed (87 passed, 2 ignored), along with
the desktop-controls fixture (9), Replay webview fixture (20), strict Clippy,
all eight Node test files, license checks, installer checks, staging, desktop
metadata, and package builds. The shared Rust workspace test suite also passed.
The snapshot kept the pre-existing 4.7 GB local Tauri build tree untouched.

On September 24, the production WebKit workspace fixture passed 75 checks and
the Replay fixture passed its scrolling, focus, save-duration, media-readiness,
rapid-selection, decoded-frame, and trim/export checks in isolated Xvfb. I
visually inspected the workspace and Replay fixture screenshots; these remain
synthetic UI checks, not owner acceptance of an installed release.

`tools/test-updater-rpm-flow.sh` passed in the cached Fedora 44 container,
including the skipped-transaction case and upgrade/rollback with retained test
history. `tools/test-updater-versioned-rpm.sh` passed the isolated real-binary
0.1.3→0.1.4→0.1.3 upgrade/rollback and executable-hash checks. Both containers
ran without network access and made no host package changes.

The five-asset local candidate bundle is under
[`local-builds/release-candidate-local-test-signature-2026-09-23`](../local-builds/release-candidate-local-test-signature-2026-09-23/STATUS.txt).
Its Fedora 44 x86_64 RPM is version `0.1.3-1.local.fc44`, SHA-256
`705e962cecf3c62ec31b6cafe11427e89e3ace627c6f8cf12794eb486398d247`.
The manifest and all five release assets verify; its signature uses the
included test-only public key, not the production update key. The candidate RPM
payload matches the installed runtime components. No package was installed in
this revalidation. A fresh installed process passed the three isolated 2× Xvfb
window-size cases. The read-only `rpm -V redunar-app` check showed only
user/group ownership flags (`nobody:nobody` on this host); installed content
hashes and modes matched the RPM payload.

The retained synthetic GLX MKV and SDL OpenGL MP4 clips both fully decoded.
They contain 1920×1080 H.264 and Opus; the MKV has 65 video frames at about
30 FPS over 2.17 seconds, and the MP4 has 125 frames at about 60 FPS over 2.08
seconds. I inspected the GLX frame: red is at the top, blue at the bottom, and
the metrics overlay is visible. Owner playback/visual acceptance remains open.

In the September 23 native-Steam retest, Stardew sound remained absent after
`SDL_DYNAMIC_API` was unset; direct Steam launch remained audible. SDL2 and
libpulse were mapped, but no PipeWire sink-input appeared. On September 24,
source review found that OpenGL interposer SDL symbol probes could leave a
`dlerror()` after successful `dlopen()` calls, causing Stardew's OpenAL loader
to reject `libpulse.so.0`. The interposer now clears its internal loader error
after successful loads. The owner reported live Stardew sound restored with
the resulting local RPM; the installed OpenGL sidecar matches the RPM payload.
Instant Replay audio remains untested.

## September 24-25, 2026 overlay renderer redesign

The in-game metrics overlay and Replay menu were rebuilt to the approved
modern reference (soft rounded panels, quiet letter-spaced labels above bright
values, thin dividers, dark-red selected duration cell with red underline,
red-outlined save button). The shared 18x24 coverage tables were regenerated
with a 16-row mono cap and a 14-row UI cap at the original baselines, the eight
palette roles were converted from the reference's sRGB values into the
renderer's linear-light tables, and the Replay menu keeps fixed Redunar
control colors plus its own near-black panel shader slot. The OpenGL interposer
Grid layout now mirrors the Vulkan row structure, and the desktop overlay
preview palette table matches the renderer roles.

Evidence on build `0.1.4` (commit `117aef7` plus these uncommitted
changes), x86_64 Linux, Mesa `lvp` software ICD for the headless pipeline
check:

- Root workspace `cargo test --offline --workspace`: all suites pass
  (redunar-capture-vulkan 68, redunar-capture-opengl 32, redunar-daemon 214,
  plus the remaining crates); the daemon socket test requires an unsandboxed
  run.
- `cargo clippy --offline --workspace --all-targets` and `cargo fmt --check`
  are clean; the Tauri `npm run check` (Node tests plus production Vite
  build) passes.
- `blend_pipeline_is_accepted_by_headless_vulkan` passes with
  `VK_ICD_FILENAMES=lvp_icd.x86_64.json`, accepting the new nine-slot panel
  shader.
- `REDUNAR_CAPTURE_PROBE_OVERLAY=1` against `/usr/bin/vkcube`: 59 frames,
  drops=0, rejects=0, overlay_status=Some(Active).
- `tools/run-opengl-overlay-acceptance.sh` (six GLX/EGL/SDL routes, isolated
  state): all Active with drops=0 and rejects=0.
- Replay menu control path via `REDUNAR_CAPTURE_PROBE_SAVE_VIA_MENU=1` with
  replay eligibility: MENU TOGGLE/MOVE/BUTTON sequence completes, replay
  copies and exports frames, phase=Completed.
- Visual inspection used the plan-level reference dumps
  (`REDUNAR_REPLAY_MENU_REFERENCE`, `REDUNAR_METRICS_REFERENCE`) and the
  OpenGL canvas dump test (`REDUNAR_OPENGL_CANVAS_REFERENCE`) compared
  against `ai-workspace/private/overlay-reference/replay-menu-reference.png`.
  These are renderer dumps from that build, not owner visual acceptance of a
  real game session.

## September 25, 2026 color-format follow-up

The earlier palette tables held squared reference RGB channels. That build sent
them unchanged to an UNORM swapchain and the OpenGL RGBA8 canvas, which made
the menu controls and dividers too dark in actual game-window captures. The
follow-up keeps the fixed palette roles and encodes them for each Vulkan
attachment format; the OpenGL canvas now encodes them before byte blending.

On build `0.1.4` (commit `117aef7` plus the uncommitted overlay changes), an
offline release build of both injected libraries with `--lib` and the daemon
capture probe completed. In isolated Xvfb, a 640×360 `vkcube` window
using the UNORM swapchain showed the injected menu after a 16-second replay
buffer, including its red selected duration and active red-outlined save
control. The captured image is under
`../artifacts/overlay-review-2026-09-25/reworked/`. This is synthetic visual
inspection; an installed app and a real game remain unverified for this build.
The OpenGL metric panel was visually inspected in a separate isolated
`glxgears` capture using the rebuilt GL library; its Replay menu was not
inspected in that run. Automated test suites from the preceding section were
not rerun for this follow-up.

## Development entry point

Runtime-efficiency regression coverage includes exclusive startup ownership,
bounded/private history and escaping, audio progress after retention fills,
nonblocking snapshot cancellation, decoded synthetic MKV/MP4 with audio gaps,
failed streamed-output cleanup, microphone-only/mixed MKV/MP4 decode with one
audible track, microphone speech through system gaps, sample-offset mixing,
malformed-audio rejection, assembler/capture independence, two-input epoch reset,
backend cycling, route/stall recovery, and microphone preference lifecycle locks.
Built WebKit fixtures cover pending/successful/failed microphone writes, keyboard
focus, narrow layouts, and the recording-folder control. Coverage also includes
cache invalidation and polling during a delayed
or failed update check. All media fixtures use private generated data. The
WebKit scripts share the native renderer default through
`output/tauri-redunar/tests/webkit_fixture.py`
and read the frontend palette definitions instead of duplicating color literals.
This coverage does not replace real-game or installed-runtime acceptance.
See the [October 2 verification report](docs/verification/2026-10-02-runtime-efficiency.md)
for the tested source manifest, current results, and remaining acceptance scope.

See [CONTRIBUTING.md](CONTRIBUTING.md) for branches, worktrees, and review.
`tools/validate.sh quick`, `full`, and `release` share the same local/CI entry
point. Full checks cover both Rust workspaces and UI; release adds the existing
compatibility/package gate. Reports and isolated targets live in the checkout's
ignored `.redunar-build/` directory. Hardware and installation remain separate.
The audio command-construction test requires `parec` to be installed
(`pulseaudio-utils` on Debian/Ubuntu). It constructs the command without starting
capture or connecting to a sound server. The shared CI setup installs this
utility alongside the audio libraries.
The synthetic playback preparation test uses FFmpeg with `libx264` to generate
a tiny H.264 Matroska file without a seek index, matching Redunar's recorded video
codec. It checks MP4 preparation, decoding, private temporary-file cleanup, and
preservation of the original. It needs no GPU, display, or real recording. FFV1
is unsuitable for this copy-video-to-MP4 path on older FFmpeg versions. If FFmpeg
is absent the test returns early; acceptance runs must have FFmpeg installed,
as the shared CI setup does. A missing `libx264` encoder fails fixture generation.
The sparse-focus fixture checks demuxed video PTS, stored MP4 fragment/sample
start times and durations, and decoded Opus audio continuity. Older FFprobe
versions can report codec-derived packet durations for fragmented H.264, so the
fixture reads the saved MP4 sample tables independently instead of treating
those estimates as container timing. The no-loss spool history fixture waits
for bounded worker progress; production queue-full drops remain unchanged.
New detailed results belong in [docs/verification](docs/verification/README.md).
See the [September 26 workflow verification](docs/verification/2026-09-26-development-workflow.md)
for the isolated build and validation entry point.
Existing dated records below describe their original builds and paths.

## Automated commands

Shared Rust workspace:

```sh
cargo fmt --all -- --check
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
```

Use `-p <affected-crate>` for focused iterations. Tauri workspace and frontend:

```sh
cargo fmt --manifest-path output/tauri-redunar/src-tauri/Cargo.toml --all -- --check
cargo test --offline --manifest-path output/tauri-redunar/src-tauri/Cargo.toml --all-targets
cargo clippy --offline --manifest-path output/tauri-redunar/src-tauri/Cargo.toml --all-targets -- -D warnings
node --test output/tauri-redunar/tests/*.test.mjs
tools/build-linux-release.sh
```

The production release/package gate is:

```sh
tools/check-tauri-release.sh
```

It builds Tauri, runs native/Node/lint checks, validates desktop/AppStream metadata
without network access, checks the staging tree and privilege boundaries, and
builds/checks an RPM when `rpmbuild` is installed. It does not install it or run
the ignored hardware/game acceptance tests. Inspect the script when changing
release behavior; do not substitute root-only Cargo success for this gate.

Installer-only checks do not install packages or contact GitHub:

```sh
tools/test-installer.sh
tools/build-tauri-deb.sh target/install-assets
tools/build-tauri-arch-package.sh target/install-assets
tools/build-tauri-opensuse-rpm.sh target/install-assets
tools/build-tauri-portable.sh target/install-assets
tools/render-public-installer.sh example/redunar /absolute/output/install.sh packaging/release-signing-public.pem
REDUNAR_GITHUB_REPOSITORY=example/redunar ./install.sh --check
```

The installer suite covers Fedora, Ubuntu/Debian, Arch, and openSUSE plans plus
old-glibc, ARM, Alpine, NixOS, and immutable-Fedora refusal paths. It also uses
an isolated local download and fake package manager to verify signed-manifest and
checksum success, corrupted-asset rejection, and invalid-signature rejection
without changing the host. Package checks establish structure, dependency metadata,
and signature consistency, not runtime compatibility on those distributions.
The release gate also requires `VERSION` to match the Tauri and RPM package
versions and appear in the signed checksum manifest. Updater unit tests reject
an absent or changed version checksum and parse installed package versions for
the native restart-versus-retry state. The WebKit Settings fixture covers both
states and automatic checks without automatic installation.
On September 23, 2026, the pending-package refresh unit tests passed and the
74-check WebKit fixture passed after manual checks began requesting a signed
channel refresh.
On September 23, 2026, updater unit tests also passed for a locally generated
signed manifest and its tampered variant, plus failed cache-metadata commit and
successful replacement of a legacy cached package. These fixtures do not use
the production private signing key or prove a public update download.
The native updater also passed an end-to-end local signed-feed test using a
temporary RSA key, `file://` metadata, actual download and checksum verification,
cached-package reuse, a newer signed release, and rejection of tampered metadata
and package bytes while retaining the verified package. This tests the native
download path without a public feed or the production signing key.
Native installer-state tests now also cover first handoff, reopening after an
unconfirmed or cancelled handoff, digest-named and legacy cache paths, restart
only after the package database reports the new version, opener failure, and
tampered cached bytes. They do not replace a real desktop package-installer run.
On September 23, 2026, the window-geometry and History-scrollbar source passed
the full offline Tauri release gate, nine focused app-preference tests, and a
75-check isolated WebKit workspace fixture. The fixture rendered a 20-entry
History list and confirmed that the scroll viewport extends into the column
gap while cards remain within the entry column; its screenshot was inspected.
The local Fedora RPM SHA-256 is
`a90a5fe703a2519d55979d84f619187fbca01dc3b580b1e934f3f4f337502a13`.
The same RPM passed `tools/test-updater-rpm-flow.sh` in an offline Fedora 44
container: a skipped transaction left 0.1.3 installed, a metadata-only 0.1.4
upgrade and rollback completed, and fixture history survived. This does not
exercise the desktop installer GUI or change the host package.
The matching release binary and, after host installation, the installed binary
both passed the isolated 2× Xvfb window-geometry probe: V5 physical sizes
1280×720 and 2560×1440, plus V6 logical size 1280×720, each opened at
2560×1440 physical pixels. The installed probe also confirmed that every fresh
process loaded `/usr/bin/redunar-tauri`; no window appeared on the active
desktop or used its saved preferences. `tools/check-installed-tauri-runtime.sh`
matched the current RPM's executable, Vulkan and OpenGL libraries, Steam
wrapper, shortcut helper, desktop entry, AppStream metadata, icon, and uaccess
rule. Host-namespace `rpm -V redunar-app` returned no mismatches. The installed
executable SHA-256 is
`c3f3160209e3eea5a6d1bdd9c986f58b4acfbdc6ea9e1ce7c5f9bf779d7fcb1e`.
The following installed Replay-listener evidence applies to the preceding RPM.
On September 23, 2026, the full Tauri release gate passed for the OpenGL,
updater, and secondary-instance source, including the local signed-feed test.
That preceding local Fedora RPM SHA-256 was
`d93ea51ce4cd62b4442759ffb8e8d83869ca3f56b2a87e07aa2bc6cd0cdaabf0`.
After its same-version host reinstall, `tools/check-installed-tauri-runtime.sh`
matched every packaged runtime component. A fresh process, PID 598952, matched
the installed executable SHA-256
`2d279a7e02aa7e6153bd1ff09c84a158c91c09e3dbf60023e8bf22b7a8e444e7`
and owned the Replay socket. A second installed process remained open while the
primary stayed running and retained the socket. The complete release gate
passed on this source after the code-level write-access guard for updater
commands. The following package transaction evidence was collected on an
earlier RPM,
`2d58e887687f2723853689b89793abe6430fcb8f883e0278bd0fce8ca649b794`.
After that RPM's same-version host reinstall, `tools/check-installed-tauri-runtime.sh`
matched the executable, both capture libraries, Steam wrapper, shortcut helper,
desktop entry, AppStream metadata, icon, and uaccess rule. A fresh
`/usr/bin/redunar-tauri` primary process loaded the new executable and owned
the Replay socket. A second installed process remained read-only for ten seconds
without a socket conflict or primary crash; the primary still owned the socket
afterward. Earlier apparent first-launch conflicts were secondary launches
while an older `/usr/bin/redunar-tauri (deleted)` process still held the socket
across reinstall. A deliberately launched second process had also triggered a
primary Tauri panic from duplicate `main` webview setup before the GTK identity
fix. The final-source isolated Fedora 44 real-binary upgrade/rollback test
passed 0.1.3→0.1.4→0.1.3 and verified executable hashes and retained local
history. This does not validate an in-game OpenGL run on the new package, a
public signed update, or a real desktop installer cancellation.
On September 23, 2026, the same Fedora 44 x86_64 host then upgraded from the
candidate 0.1.3 RPM (SHA-256
`2d58e887687f2723853689b89793abe6430fcb8f883e0278bd0fce8ca649b794`)
to an isolated real-binary 0.1.4 RPM (SHA-256
`cdfeb38880bec9ce3eeaa54335edce03333a50346a5fe0577db1a06aacc4d3e2`).
The running 0.1.3 process kept its original executable hash until stopped;
a fresh 0.1.4 process matched the newly installed binary. The host then rolled
back to the exact 0.1.3 RPM, removed it, and reinstalled it. RPM removal cleared
the packaged binary while the Redunar user-state directory remained present
with the same 41 files and 286,276,492 total bytes before and after. Final
installed-payload comparison passed, and a fresh 0.1.3 process matched the
installed executable and owned the Replay socket. This validates package
transactions and restart on this host, not the in-app signed download or a
desktop-installer cancellation.
Earlier on September 23, the gate passed for the installer-state source; that
local RPM SHA-256 was
`23d24e901f49f48e38df5fa44788168d015f5bd468082acea2f71d9fa0a88abd`.
Earlier on September 23, the gate also passed for the atomic-cache source;
that local RPM SHA-256 was
`3492e955b82a31249d714016d241baca92fca1ea556ef3b1ec689380b99e0f2a`.
Earlier on September 23, the full release gate also passed for the OpenGL
back-buffer and updater-refresh source; that RPM SHA-256 was
`f7fbd2df7eb9824ef2e75343fafc4a8fe65309325ded192ccc8fc74296848ade`.
On September 23, 2026, the full Tauri release gate and 74 WebKit workspace
checks passed for the updater working-tree build. Its local Fedora RPM SHA-256
is `7f8dbbeff27dfd257d5d86d899379361d528df7254e27b6791e49cd3a16b224f`.
After a same-version reinstall, `tools/check-installed-tauri-runtime.sh`
matched every payload component and a fresh `/usr/bin/redunar-tauri` process
remained active. On September 23, 2026, `tools/test-updater-rpm-flow.sh` also
passed an offline Fedora 44 container check: the package manager installed
0.1.3, left it intact when no upgrade transaction was started, upgraded a
synthetic RPM to 0.1.4, and rolled back to 0.1.3 while retaining local history.
The synthetic RPM changes package metadata only. On September 23, 2026,
`tools/test-updater-versioned-rpm.sh` also passed: it rebuilt the native Tauri
app as 0.1.4 in an isolated source snapshot, packaged it, and checked that an
offline Fedora 44 RPM upgrade and rollback installed the exact executable
payload from each RPM while retaining local history. Both tests require a
locally built Fedora RPM and a cached Fedora 44 Podman image; they pull nothing
and change no host packages. The versioned test additionally requires local
build dependencies and can take several minutes. Neither test validates the
public update signature, desktop installer cancellation, running-app restart,
or a host package-manager transaction with real system dependencies.

## Built interface and media

After building, run the relevant WebKitGTK fixture checks:

```sh
xvfb-run -a python3 output/tauri-redunar/tests/workspace-webview.py
xvfb-run -a python3 output/tauri-redunar/tests/replay-webview.py
```

These use isolated frontend fixtures, not real user sessions. Inspect affected
screens at normal and compact widths and with keyboard input. Check empty,
loading, failed, inherited/custom, unavailable, and long-text states. Review
computed styles from the built page, not a CSS file that may be overridden.

To check native window restoration at 2× scale without opening Redunar on the
active desktop, run the isolated Xvfb probe after a production build:

```sh
dbus-run-session -- xvfb-run -a -s '-screen 0 3200x2000x24' python3 tools/check-tauri-window-geometry.py /absolute/path/to/output/tauri-redunar/src-tauri/target/release/redunar-tauri
```

The probe supplies temporary V5/V6 app-preference files, confirms actual X11
window dimensions, and removes its temporary state after exit.

For the real native playback command/stream/WebKit path:

```sh
cargo build --release --offline --manifest-path output/tauri-redunar/src-tauri/Cargo.toml --example replay_webview_probe
xvfb-run -a output/tauri-redunar/src-tauri/target/release/examples/replay_webview_probe
```

Set REDUNAR_PROBE_PLAYBACK_SECONDS=32 for an optional sustained, muted native
playback check (bounded to 60 seconds). It exercises the production player after
filmstrip completion and rejects a stalled playback clock. An optional source
clip is copied privately and never modified. Synthetic sparse-transition tests
also cover 90 FPS / 1 FPS / 90 FPS, continuous audio with a silent middle,
MP4 fragment holds, and fixed-rate timestamp recovery after pauses.

The default probe generates isolated H.264/Opus media and requires a decoded frame
and filmstrip samples. An optional clip argument uses a private copy of that
recording. Do not log private stream tokens or modify source clips. Fixture
success does not establish real-game performance or every distribution's codecs.

## Retained engineering diagnostics

The following examples are intentional engineering tools. Keep them unless their
capability has a documented replacement. They are outside the ordinary automated
gate: select one for a specific question, record the build and host, and interpret
only the property it checks.

Synthetic and isolated diagnostics are safe for routine development:

```sh
cargo run --release --offline -p redunar-daemon --example replay_foundation_self_test
cargo run --release --offline -p redunar-daemon --example replay_profile
cargo run --release --offline -p redunar-daemon --example replay_store_profile
cargo build --locked --offline --manifest-path output/tauri-redunar/src-tauri/Cargo.toml --example desktop_controls_probe --target-dir .redunar-build/checks/tauri
dbus-run-session -- xvfb-run -a .redunar-build/checks/tauri/debug/examples/desktop_controls_probe
```

`replay_foundation_self_test` uses temporary state and deliberately reports that
hardware encoding and decoded playback are unverified. `replay_profile` measures
bounded in-memory ring insertion with synthetic packets. `replay_store_profile`
writes and removes one synthetic 16 MiB container under the temporary directory.
`desktop_controls_probe` uses an isolated D-Bus/Xvfb session, fixture preferences,
and a pipe-only shortcut helper; it does not open input devices or change host
settings. The native tray unit fixtures create private D-Bus daemons and fake
watchers, covering exported icon/menu properties, typed actions, denied or absent
registration, another app's item, host/bus loss, owner replacement with delayed
replies, repeated connection cleanup and malformed requests. They require
`dbus-daemon`, no desktop tray provider or input device. Full validation runs them.

The explicit `native_tray_probe --desktop` example briefly registers a temporary
icon with the real session host without changing preferences or opening games.
It is separate from ordinary tests; observe the icon/menu and confirm cleanup.

These live probes require an explicit, scoped hardware run:

| Probe | Command and scope |
| --- | --- |
| System-output audio | `cargo run --release --offline -p redunar-capture-audio --example output_audio_live_probe` resolves the actual default output, starts temporary silent playback on that exact sink, captures ten Opus packets from its monitor through the selected PulseAudio/PipeWire backend, and terminates both children. It does not capture microphone input or retain audio. |
| Vulkan capture | Build `redunar-capture-vulkan`, then run `cargo run --release --offline -p redunar-daemon --example capture_probe -- /usr/bin/vkcube`. It launches the absolute target through the private layer and cleans isolated state. `REDUNAR_CAPTURE_PROBE_OVERLAY=1` checks rendered overlay submission; `REDUNAR_CAPTURE_PROBE_REPLAY_CANDIDATE=1` requests source eligibility; `REDUNAR_CAPTURE_PROBE_REQUIRE_REPLAY_CANDIDATE=1` makes missing eligibility fail; `REDUNAR_CAPTURE_PROBE_REPLAY_ENCODE=1` performs a real Vulkan Video encode. WSI, frame count, dimensions, FPS, metrics, preset, corner, opacity, scale, and timeout have bounded `REDUNAR_CAPTURE_PROBE_*` overrides in the example source. |
| OpenGL capture/overlay | Build `redunar-capture-vulkan`, `redunar-capture-opengl`, `redunar-steam-launch`, and the OpenGL examples, then run `REDUNAR_CAPTURE_PROBE_OVERLAY=1 REDUNAR_CAPTURE_PROBE_PRESET=fps-only cargo run --release --offline -p redunar-daemon --example capture_probe -- /absolute/path/to/target/release/examples/glx_probe` and repeat with `egl_probe`, `sdl_gl_probe`, and `sdl_renderer_probe`. Both SDL probes deliberately use glibc's versioned `dlsym` directly, bypassing preload lookup like managed-runtime deep binding. The renderer probe also calls `SDL_SetHint` before `SDL_Init`, verifying that Redunar repairs SDL's dynamic table during its initialization event instead of depending on call order or timing. Add `REDUNAR_CAPTURE_PROBE_STEAM_BRIDGE=1` to exercise the one-shot native Steam wrapper handoff and session-private `LD_PRELOAD` path. Each bounded target requires non-empty frame telemetry and `overlay_status=Some(Active)`. Preset, layout, palette, branding, metrics, corner, opacity, scale, and frame-dump paths have bounded `REDUNAR_CAPTURE_PROBE_*` overrides. FPS only must remain panel-free; Detailed and Custom must omit unavailable hardware instead of fabricating values. Frame dumps support visual opacity inspection, but the probe does not automatically compare blended pixels. These probes do not validate Replay source, Flatpak capture, or broad real-game compatibility. |
| KMS discovery/planning | `cargo run --release --offline -p redunar-capture-kms --example kms_replay_probe` enumerates local DRM outputs and constructs 60 FPS plans. It does not capture or encode frames. |
| KMS DRM read | `cargo run --release --offline -p redunar-capture-kms --example kms_drm_read_probe -- 1 DP-2` opens the selected card/connector read path and reports its active dimensions. Replace the example arguments with the intended output. |
| KMS encoder setup | `cargo run --release --offline -p redunar-capture-vulkan --example kms_encoder_probe` creates the production KMS Vulkan Video encoder at its fixed 2560x1440@60 diagnostic request. |
| KMS live replay | `cargo run --release --offline -p redunar-daemon --example kms_replay_live_probe -- 8` performs local KMS capture and hardware encode for the requested frame count. |

The KMS/DRM examples are retained exploratory diagnostics, not the production
per-game Replay path and not evidence of supported desktop capture. The
`redunar-platform` `prepare_capture_layer` example is a low-level manifest helper
for a supplied session directory and layer library; normal sessions prepare this
through the service and should not need the example directly.

`tools/run-opengl-overlay-acceptance.sh` runs the bounded GLX, EGL, SDL OpenGL,
and SDL renderer matrix across direct and native-Steam-wrapper launch plans. It
checks non-empty telemetry, zero transport drops/rejects, and active overlay
submission for Grid, Ribbon, Telemetry, FPS only, Detailed, and Custom paths.
It opens short local graphics probe windows and is therefore an explicit local
hardware acceptance run, not an ordinary unit-test command or a real-game test.

`tools/run-opengl-replay-foundation.sh` runs a diagnostic-only asynchronous PBO
readback matrix over the same direct and native-Steam-wrapper routes. It requires
bounded source metadata, completed copy proofs, correct byte counts and sampled
checksums, active overlay submission, and zero transport drops/rejects. The
producer uses three slots, polls fences with a zero timeout, drops work when all
slots are busy, and restores framebuffer, pixel-pack buffer, alignment, row
length, and skip state before returning to the game. This test does not export a
DMA-BUF, feed the encoder, create a Replay buffer, or establish OpenGL Replay
support. The September 22, 2026 development-host run completed all six routes
with 179 presentation frames each and 61 or more completed readbacks per route.
The same run also retained zero drops/rejects after adding a private producer
reply socket, bounded release polling, and exact
`GL_EXT_memory_object`/`GL_EXT_memory_object_fd` capability checks. A local
`glxinfo` query confirmed both extensions for core, compatibility, and GLES
profiles on the AMD Radeon RX 6800 XT using Mesa 26.2.2.

`tools/run-opengl-replay-acceptance.sh` is the production Replay matrix. It
uses GLX and SDL OpenGL targets through direct and native-Steam-wrapper launch
plans, covers metrics shown and hidden plus 30 and 60 FPS, and requires a real
H.264 clip from the DMA-BUF/Vulkan Video path. `ffprobe` must report the source
dimensions, requested cadence, decoded frame count, and YUV420 output. The
probe target must actually create the requested dimensions and frame count;
exported cadence must remain within 85% of the active presentation window,
while capture reports zero transport drops and rejects. The matrix opens four
short local graphics windows, one sustained 1080p/60 window, one gated
1080p/120 window, a live 640x360 to 1280x720 resize, and six GLX context
replacements in one session, two simultaneous GLX windows with independent
contexts, and four one-shot OpenGL failure stages. The resize must save only
the final codec epoch; the replacement route exceeds the four-pool cap to
prove destroyed contexts return their pool budget. The second window swaps
every frame but must not inflate game metrics or change the Replay source.
The held-release route deliberately withholds all six producer ownership
tokens and requires Replay-only drops while game presentation continues;
one September 23 run recorded 318 producer Replay drops, 599 presentation
frames, and zero transport drops/rejects.
The OpenGL unit suite also drops the daemon side of a private datagram pair,
then drives 10,000 producer presents. It requires bounded metric batches and
clean producer exit. This models daemon loss, while held-release saturation
models the separate case where exported Replay slots receive no daemon
acknowledgements. Neither route promises hot attachment to a restarted daemon.
This is an explicit local hardware acceptance run. The original 13-route
matrix passed on the AMD/RADV development host on September 23, 2026 with
zero transport drops or rejects. A September 23 rerun including held-release
backpressure passed all 14 routes, with 319 producer-only Replay drops in the
new route. The full Tauri release gate also passed for
that local working-tree build based on `659b519`. A later cleanup-only update
to EGL destruction and partial overlay setup passed the six-route overlay
matrix, focused OpenGL tests and strict Clippy, and a fresh production release
build. Its RPM SHA-256 is
`32e180691cff0712b8358739f964e073a34a75054604636ebb3a655320d072b0`.
That earlier RPM passed the installed-runtime hash check, and a fresh
`/usr/bin/redunar-tauri` process remained active after installation. The later
September 23 production Replay build passed `tools/check-tauri-release.sh`,
the 14-route capture matrix, and three consecutive GLX/SDL/fullscreen
production Replay matrices. A final release-acknowledgement fix retained
daemon tokens across brief capture-lock contention; 30 OpenGL unit tests,
strict Clippy, the production Replay matrix, and the full Tauri release gate
passed afterward. That OpenGL-capture milestone RPM SHA-256 was
`e27369fca655794a202eb5b273b2d39cb9e2301e9541c229e93c2cdb957b3f3c`.
After reinstalling that RPM, `tools/check-installed-tauri-runtime.sh` matched
all installed payload components, including the OpenGL sidecar. A fresh
`/usr/bin/redunar-tauri` process remained active, with `/proc` confirming it
loaded the installed executable. RPM packaging strips the binaries, so compare
installed files to the extracted RPM payload, not to the unstripped build.
That installed OpenGL sidecar SHA-256 was
`b155051d30e966fdbbd8c60ba09a19fda44d8efc8f8f42c56815cb21e8c38eec`.
These probes do not replace owner testing in Stardew Valley or another real
OpenGL game.

`tools/run-opengl-production-replay.sh` additionally starts the daemon-owned
Replay export pump through the normal profile launch plan, waits for native
Buffering and Saving, checks menu pointer control and release, and requires a
committed local clip. It runs GLX 1080p30/MKV with metrics visible and SDL
OpenGL 1080p60/MP4 with metrics hidden.
The GLX case chooses duration and format through the menu and saves from it;
SDL uses the app service action after an in-menu MP4 choice. The fullscreen
route saves through the bounded shortcut control protocol. Each case toggles
metrics visibility while Replay remains buffering and publishes Moment saved
only after native commit. Both output formats contain H.264 video and Opus
audio and decode
without errors. Both GLX and SDL cases check decoded top-red/bottom-blue pixel
orientation as well as frame rate/count, duration, and a keyframe. The
September 23, 2026 isolated AMD/RADV run passed with zero capture
drops/rejects. This verifies the actual session and save worker, unlike the
direct encoder matrix above; synthetic clips still need owner visual approval.
Set `REDUNAR_OPENGL_REVIEW_DIR` to an absolute local directory to retain the
validated MKV/MP4 clips and first-frame PNGs after the probe's isolated state
is cleaned up. A September 23 rerun exposed an incorrect OpenGL enum: the
export path used `0x0400` (`GL_FRONT_LEFT`) for `GL_BACK`, producing black SDL
MP4 video. The corrected `0x0405` back-buffer selection passed 31 focused
OpenGL tests, strict Clippy, the three-route production save test, the
14-route direct/native-Steam Replay matrix, and the full offline Tauri release
gate on working-tree build `659b519` plus the fix. The rebuilt local Fedora RPM
SHA-256 is `17a80b799f992ea4007334c4b00e59505c785f6aa8881eb8676be429f69bdfda`.
Review artifacts are under `target/opengl-review-2026-09-23-fixed/`; their
GLX MKV and SDL MP4 SHA-256 values are respectively
`2752b7cb9c84693f1bac6f2f0ec837f8670e1c77fbee4075e4d05d9d16250a42`
and `b949f3af577371e76fb431bdadd89668761ceee04de4ae5d258e80215aeb7798`.
The rebuilt package requires a separate installed-runtime check before any
claim that the host is running this fix.
One-minute 1080p30 GLX and 1080p60 SDL production runs on the same host
completed September 23 with 1,821 and 3,581 encoded packets, zero capture
drops/rejects, and no residual probe FD/thread growth after session teardown.
The GLX production probe also requests and observes an X11 fullscreen/restore
transition; the recorder must reset for both 2560x1440 and 640x360 epochs and
save a decodable clip without capture drops.

## Regression areas

- **Profiles:** fresh/previous-version state, inheritance, stale revisions, unsaved drafts
  across games, partial failures, unrelated preference saves, and restart.
- **Visibility:** start a supported game with metrics hidden, show/hide repeatedly,
  and preserve runtime availability. A recording-settings lock must not block
  visibility-only changes; custom per-game values override inherited global ones.
- **Library:** bounded import, duplicates, ambiguous identity, missing executable,
  retained artwork after uninstall/cache removal, separate poster/banner roles.
- **Replay:** rapid selection/cancellation, rail scroll, native temporary-file
  exhaustion/errors, decode/seek deadlines, resize, save/export failure, source
  preservation, deletion boundaries, and final empty state. Fake two-producer
  fixtures cover overlapping/lower wire sequences, delayed/out-of-order GPU
  completion, duplicate confirmation, failed acknowledgement retry, producer
  exit, foreign sessions, smaller replacement sources, device recreation,
  rejected-import cleanup, copied telemetry identity, and capacity backpressure.
  Private socket fixtures cover saturated shutdown, producer incarnation reuse,
  and full pathname limits before runtime-file creation.
- **Menu:** dedicated native document, transparent exterior, preview isolation,
  focus/Escape/outside-click, hidden main window, and close without app shutdown.
- **Shortcuts/tray:** empty/menu-only/save-only bindings, duplicates, persisted
  clearing, hidden-window dispatch, no disabled tray icon, live preference changes,
  missing providers/access, and recovery of a hidden window after tray loss.
- **History:** raw extrema/spikes, null gaps, real timestamps, units, axis/trace
  alignment, both CPU/GPU series, nearest-sample selection, previous-version and partial data.
- **Lifecycle:** direct/forwarded launch, duplicate instance, launch lock after End,
  natural exit, one history write, cleanup, unsupported capability, and restart.
- **Recovery:** fixture-only exact restoration, ownership conflicts, partial
  application, and crashes. The Tauri application must keep privileged actions
  outside the webview.

The [audio behavior](REPLAY.md#audio-behavior) needs default-output discovery,
PulseAudio compatibility, direct PipeWire fallback, route-change, stall, and
backend-failover checks. Captured output can include other applications.
Media-tool tests cover executable discovery, provider-neutral encoder checks,
cached success/retried misses, missing codecs, timeouts, and bounded probe output.
The RPM gate checks file/capability requirements without pinning a multimedia
package name or declaring replacement. Remaining release work is in ROADMAP.

## NVIDIA beta first hardware test

The [October 2 readiness report](docs/verification/2026-10-02-nvidia-readiness.md)
records the tested source, automated evidence, and remaining lifecycle limits.
The [handoff and compatibility follow-up](docs/verification/2026-10-02-handoff-compatibility.md)
records the repaired producer ownership and completed release/package gate.
The [October 4 attribution report](docs/verification/2026-10-04-nvidia-game-device.md)
records the later hybrid discovery and game/encoder identity changes, fake-hardware
regressions, and matching build checks; it adds no NVIDIA hardware qualification.

This is an explicit human hardware test, outside ordinary automated validation.
The official package includes the Beta access switch. Install the exact approved
candidate, enable **Beta access** and separately **Debug log**, fully quit and
restart Redunar, then launch a new session. Check that Settings says logging is
active. A checked saved preference alone does not confirm that the log opened.

Start with x86_64 Linux, one identifiable NVIDIA render GPU (a known Intel/AMD
render GPU may coexist), 8-bit SDR, fixed 1080p60/Balanced. Exercise native Vulkan
and supported desktop OpenGL separately. Record package version, actual
executable/sidecar hashes, driver version, compositor, and test scope. The log
records the running executable hash when readable, but that does not verify the
sidecars. No driver/GPU model is certified by a fixture or an NVML reading.

Before an owner-approved game run, the bounded generated-scene acceptance runner
can exercise the production Tauri supervisor with isolated state and the saved
Beta preference established before service construction:

```sh
tools/run-tauri-vulkan-replay-acceptance.sh --nvidia-beta
REDUNAR_TAURI_REPLAY_FORMAT=mp4 tools/run-tauri-vulkan-replay-acceptance.sh --nvidia-beta
```

These commands open a graphics window, create a real encoded clip, decode its
video with FFmpeg, and clean their private state. They require explicit hardware
authorization and the documented matching release artifacts. The lower-level
capture probe also has an explicit NVIDIA opt-in; inspect its usage before a
scoped run. KMS examples are not NVIDIA production acceptance.

The Tauri Replay runner compiles the independently implemented
`tools/fixtures/vulkan_scene.c` with a C compiler and the local Vulkan/XCB
development headers (`pkg-config vulkan xcb`). It explicitly requests Vulkan 1.1,
runs at bounded dimensions/frame count, and destroys its graphics resources
normally. Distro vkcube variants that request Vulkan 1.0 cannot exercise Replay.
The live test has a 45-second session deadline and a separate 75-second process
deadline after compilation; its failure guard ends the coordinator before
dropping isolated state. Compilation itself has no hardware deadline.
The fixture permits up to ten 20-ms retries of the spool's explicit busy
snapshot response; all other save errors fail immediately, and success still
requires an indexed fifteen-second clip and a clean FFmpeg video decode.

`tools/check-vulkan-layer-binding.sh /absolute/libredunar_capture_vulkan.so`
checks the packaged Linux cdylib with competing loader exports and no hardware.
The release gate runs it after the compatibility build. RPM payload checks use
a private temporary database and complete file-list queries before filtering,
so neither a missing host RPM database nor early pipe closure bypasses a gate.

For the tester's normal app session, first check metrics and their unavailable
states, then a short capture/save/playback/end cycle. Check video and audio in
Redunar and an external player. Next test switching windows, resize, shortcuts,
ending, and relaunching; inspect decoder errors and resource cleanup separately.
Check source identity availability and encoder UUID matching; do not infer game
GPU ownership from GPU 0, `card0`, or `renderD128`. Unknown topology and multiple
NVIDIA render GPUs remain deliberately gated. Stop the run on device loss/reset,
stalled presentation or end, repeated recovery, corruption, mixed resize epochs,
or growing retained resources. Report a capability rejection as a finding;
do not bypass a gate just to make recording start.
Repeated swapchain replacement can exhaust the bounded presentation retirement
budget when old images never provide independent completion proof. Treat this
as an explicit Replay-unavailable result; game presentation should continue.
See [the lifetime limit](REPLAY.md#capture-and-encoding). Do not infer unlimited
resize recovery from synthetic copy-fence or queue-idle fixtures.

Use **Open log folder** after reproducing the problem. Logs remain local and
sharing is explicit. Review the relevant attempt and startup lines before
sharing; general messages can contain game/session details. Include actual
failure and successful-stage lines, plus aggregate progress around the problem.
Wall-clock prefixes may be removed while retaining monotonic elapsed times.
Queue-drop summaries mean some diagnostics were lost, not that the game dropped
those frames. Logging cannot replace Vulkan validation or a hardware test.

## Installed app and owner-controlled runs

On September 26, 2026, working tree `fc6acfd` plus the Variable FPS Replay-menu
telemetry fix passed targeted offline Cargo checks and strict Clippy for the
shared crates and Tauri workspace, formatting, and the offline Debian 12
release build. A local Fedora RPM was built with SHA-256
`ecc305c677f99f1fd6e20b8f386a9eaef9a6c1c783af1ec19e78f06f075119d4`.
At build time the package retained version 0.1.9 and had not been installed or
checked in a real game. Automated tests were not run for this change. Later on
September 26, the same RPM was installed; the installed Tauri binary and both
capture sidecars matched its package digests, and a fresh installed app process
started. The owner launched ARC Raiders and reported that Ctrl+Alt+Z displayed
the Replay menu. This is a user-observed menu check, not a full Replay or
cross-distribution acceptance run.

```sh
tools/check-installed-tauri-runtime.sh
```

This read-only check compares installed binary/sidecars with the release build.
After installing an update, fully quit Redunar, including its tray process, then
reopen it. A new RPM file does not replace an already-running executable. Record
build/version, host, test scope, and failures; do not hardcode old passing counts
as acceptance criteria.

Use [REAL-GAME-TESTING.md](REAL-GAME-TESTING.md) for the small agreed game matrix
and targeted Vulkan/session/replay runners. Do not run a blanket `--ignored`
test suite: select explicit hardware tests and understand their effects.

Old recipes and dated results are not part of the active Tauri release checklist.
