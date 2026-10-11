# Redunar architecture

Current source map and ownership contract, reviewed September 23, 2026.
The production desktop is Tauri. This document describes the current ownership
boundaries, not a proposal to create another desktop frontend or daemon process.

## Runtime ownership

```text
Tauri webview: output/tauri-redunar/ui/
    | typed native commands; displayed data and local drafts
Tauri host: output/tauri-redunar/src-tauri/src/
    | one app-lifetime RedunarService::for_tauri()
Shared service: crates/redunar-daemon/ + core/platform models
    | owned session, monitor, capture, replay, persistence
Private Vulkan layer / OpenGL interposer / Steam bridge / same-user shortcut helper
    | bounded local protocols and literal process arguments
Supported game and local Linux interfaces
```

The shared service is linked into the Tauri process. “Daemon owns state” is a
logical ownership rule, not a claim that Tauri talks to a standalone D-Bus
service. The native host coordinates commands, windows, workers, and supervision;
the webview never writes hardware, accepts arbitrary playback paths, or decides
that capture succeeded. A second app cannot take over an occupied live replay
coordinator. Hidden webviews must not stop native supervision or shortcuts.

Before cleanup or mutable startup, the Tauri service acquires an exclusive
kernel lock on `backend-owner-v1.lock` in its state directory. All service clones
retain the lease until their coordinator/listener resources are dropped. The lock
file is never unlinked; process exit releases it. Contention creates a read-only
secondary, and acquisition errors fail closed. A live legacy Replay socket also
withholds writes for compatibility with older builds that do not take the lock.

| Component | Responsibility |
| --- | --- |
| `redunar-core` | Typed identities, profiles, measurements, capabilities, and compatibility models |
| `redunar-platform` | Linux telemetry/process adapters, literal launch planning, private Steam bridge |
| `redunar-daemon` | Service state, catalog/preferences/history, monitor, capture receiver, replay lifecycle and store |
| `redunar-capture` | Versioned bounded telemetry/control contracts |
| `redunar-capture-vulkan` | Game-local presentation measurements, metrics rendering, bounded GPU frame export |
| `redunar-capture-opengl` | Launch-scoped GLX/EGL/SDL presentation measurements, metrics rendering, and guarded desktop-OpenGL Replay export |
| `redunar-capture-audio` | Default-output and optional microphone capture through PulseAudio or PipeWire, with bounded Opus packets; see audio behavior in REPLAY |
| `redunar-hotkeys` | Same-user bounded keyboard shortcut helper |
| `redunar-capture-kms` | Experimental diagnostic, excluded from production Replay |

Replay microphone helper ownership and route recovery live in
`crates/redunar-daemon/src/game_session/microphone.rs`. The session coordinator
serializes mode writes with launch ownership. Runtime audio buffers own retained
encoded packets; the spool assembler selects the video window before requesting
its bounded audio snapshots. `replay_audio_mix.rs` decodes and mixes those
snapshots on the assembler thread, outside capture/runtime locks. Capture utility
arguments, discovery deadlines and helper cleanup stay in `redunar-capture-audio`.

Follow native command registration and `RedunarService::for_tauri()` for
production behavior. The Tauri service owns the active feature and privilege
boundary; compatibility data must never activate an unsupported capability.

The Linux Vulkan cdylib binds its internal function references locally. The
loader exports the same Vulkan command names; negotiation and proc-address
lookup must return Redunar hooks even when a game linked the loader globally.
Calls to the next layer still use its captured dispatch pointers. The ordinary
competing-symbol fixture in the package gate verifies this without a GPU.

## Native source map

Paths below are relative to `output/tauri-redunar/`:

| Flow | Main files |
| --- | --- |
| Commands and app lifetime | `src-tauri/src/main.rs`, `backend.rs`, `runtime.rs`, `window_lifecycle.rs` |
| Launch and supervision | `src-tauri/src/launch_plan.rs`, `sessions.rs`, `session_lifecycle.rs`, `background_start.rs`; `crates/redunar-platform/src/steam_session_bridge.rs` |
| Catalog/profile saves | `src-tauri/src/catalog.rs`, `profiles.rs`; `ui/game-drafts.mjs` |
| Installation and artwork | `src-tauri/src/installation.rs`, `artwork.rs`; matching `ui/game-*.js` |
| Playback/export/metadata | `src-tauri/src/playback.rs`, `clip_export.rs`, `clip_metadata.rs`, `media.rs` |
| Replay menu and shortcuts | `src-tauri/src/hotkeys.rs`; `crates/redunar-hotkeys` (pointer capture); `crates/redunar-capture-vulkan/src/overlay.rs` (in-game menu render); `ui/replay-menu-view.mjs` (app preview) |
| Tray preference/lifecycle | `src-tauri/src/tray.rs`, `tray_native/`, `main.rs` |
| Signed updates and installer state | `src-tauri/src/updates.rs`, `runtime.rs`; `ui/app.js` |
| Application preferences and Beta access opt-in | `crates/redunar-daemon/src/app_preferences.rs`, `src-tauri/src/runtime.rs`; `ui/app.js` |
| NVIDIA beta GPU metrics | `crates/redunar-nvidia-nvml`, `crates/redunar-platform/src/linux.rs`, `crates/redunar-daemon/src/monitor.rs` |
| NVIDIA beta Replay candidate | `crates/redunar-daemon/src/replay_encoder.rs`, `crates/redunar-capture-vulkan/src/replay_video.rs` |
| Real-data mapping | `ui/native-data.mjs`, `app.js` |
| History coordinates and inspection | `ui/history-timeline.mjs`, `history-chart.css` |
| Clip selection and previews | `ui/replay-requests.mjs`, `replay-loading.mjs`, `replay-filmstrip.mjs` |

Use [DESIGN.md](DESIGN.md) for component styling and
[REPLAY.md](REPLAY.md) for detailed media behavior rather than duplicating them.

The native updater verifies a signed release manifest, including the version
file and selected package checksum, before retaining a package in private cache.
Cache replacements use digest-named package and signed metadata files, and
commit the pending metadata last, so a failed refresh leaves the previous
verified package usable. Existing
unsuffixed cached packages remain readable until replaced or consumed.
Automatic checks preserve an interrupted installation. An explicit manual check
refreshes the signed channel and can supersede a cached package only with a
newer verified release; a channel failure retains the verified cached package.
Repository-root `tools/write-release-version.sh` and
`tools/sign-release-assets.sh` create and sign the release metadata.
Settings only requests checks or installation. The native host invokes a fixed
polkit helper; the helper snapshots cached inputs into a root-owned temporary
directory and verifies the signed manifest, version, and package checksum
before calling the distro's package manager with fixed arguments. The webview
cannot choose a package path or command. The host queries the package database
read-only to distinguish an interrupted update from a completed install that
still needs an app restart.
The Tauri backend creates a read-only service for a secondary process when
another Redunar owns the login-session Replay socket. That secondary service
does not start a competing Replay control listener. Update checking and
installation also require primary write access because they mutate the shared
update cache. The secondary GTK application is non-unique, so launching
it cannot reactivate the primary Tauri event loop and
repeat the primary webview setup. The primary retains the installed desktop
application ID.
Window geometry is persisted as logical desktop dimensions. The native host
converts Tauri's physical inner size on save and restores a logical size, so a
HiDPI scale does not shrink the next window. The app-preferences V6 reader also
retains the legacy physical-pixel marker from V2–V5 through unrelated settings
writes until geometry is saved in logical units.

## Catalog, profiles, and installation

A game has a stable local ID, validated executable/arguments, optional working
directory, match rules, and optional launcher metadata. Steam identity helps
launch and artwork lookup; it does not replace the local game record.

Installed-game import reads bounded Steam manifests and supported direct native
XDG Game desktop entries. It is read-only until the user imports candidates.
The import command re-runs discovery and resolves reviewed candidates by stable
source identity, so a reordered scan cannot silently import a different game.
Running-process discovery remains internal to monitoring/supervision; its helper
processes are not Library import entries. Identity ties remain ambiguous.

Global defaults resolve through explicitly inherited or custom per-game values.
Do not flatten inherited values on save. Native saves reject stale drafts;
unrelated game/launch/preference updates must preserve pending edits. Compatibility
fields may remain persisted without reappearing as controls.

Tauri exposes frame-metric collection, overlay visibility, and the bounded
layout/palette settings owned by the global profile. Supported launches request
replay automatically. Replay settings and shortcuts remain separately configurable.
Global overlay visibility affects the current game when inherited; its custom
per-game setting takes precedence. A visibility-only save must not be rejected
because unchanged recording settings are locked.

Installation checks and local artwork are separate from catalog persistence.
A missing game is retained, with launch rejected before runtime preparation.
Poster and banner are separate artwork roles. Valid Steam-cache images are
copied to Redunar's private cache and survive loss of the Steam originals. Steam's
600x900 poster remains preferred across cache layouts; when it is absent,
discovery deterministically selects a safe poster-shaped local raster by dimensions,
resolution, and filename hints. No network artwork lookup is required. See native
artwork/installation code for bounded paths, allowed formats, identity, and
invalidation rules.

Configured native Steam games can also start directly from Steam. The existing
app-specific wrapper first claims an already-prepared one-shot activation; if
none is available, it asks the app-lifetime `steam-session-v1.sock` listener to
prepare the unique imported game's effective profile. SO_PEERCRED verifies the
same-user PID, and the native host compares `/proc/PID/exe` with its packaged
wrapper. That live invocation proves the option was used even before Steam
persists an edited field. An ambiguous/missing local identity or busy session
is rejected. The app never receives or re-executes Steam's game argv.

When the listener is absent, the wrapper starts only its sibling `redunar-tauri
--steam-background` as a same-user transient service through `/usr/bin/systemd-run
--user`. The user manager, rather than Steam's game reaper, owns the app process.
A process-group change alone does not prevent the reaper adopting the persistent
app and waiting forever after the game exits. No scope or direct-child fallback
is permitted. Only allowlisted desktop/audio environment values are forwarded literally;
game library/identity/capture overrides are removed from the launcher and service.
An unavailable user manager leaves the original game command usable; an already
open Redunar can prepare capture without this startup dependency.
Startup and request I/O each have ten-second bounds and failures
leave the original game command usable. Concurrent starters retain the existing
backend-owner lease; a background secondary exits before GTK setup. Automatic
Steam startup keeps the main window hidden and provides Open/Quit in the tray,
including a temporary icon when Close to tray is disabled. This exception does
not change the saved preference. Opening Redunar removes the temporary icon and
retains the app; enabled Close to tray retains its normal icon. An unavailable
tray provider/registration requests a minimized window without activation, then
removes a disabled preference's temporary icon once that fallback is reachable.
After a game exits, successful session cleanup and history recording emit a native
completion edge. An automatic owner exits when Close to tray is disabled and the
user has not used its main window or chosen Open. Native button/key/touch/scroll
input and explicit Open actions retain the owner; compositor focus and passive
pointer movement do not. Manually started owners, retained windows and enabled
Close to tray remain running unless the user closes their foreground window
during a live owned game. That close creates/restores the tray entry and hides
only after verifying usable registration, without ending the session or Replay.
Closing relinquishes foreground ownership, including manual startup: disabled
Close to tray quits after successful natural completion; enabled keeps the tray
owner. Open cancels the pending close/exit and restores saved icon visibility.
An unavailable tray keeps the window reachable with an error. Duplicate close
tickets supersede older requests, and input/Open cancels pending parking.
Cleanup/history or preference
read failures reveal the window for attention. Completion carries a session
generation; shutdown claims the session lock before quitting so stale completion
cannot terminate a newer game. No extra polling worker or persisted flag is added.
Manual startup creates an icon only when Close to tray is enabled. Input
observation covers the native fallback window and webview without consuming events
or retaining an AppHandle cycle. If observation setup fails, automatic exit is
disabled. Startup neither changes Close to tray nor counts its own map or focus
as user interaction; actual minimize/focus policy belongs to the compositor.
The request listener stops on shutdown, removes
only its own socket inode, and does not attach to already-running games.
Startup cleanup reaps only the short service-manager client, never the app or an
existing owner. The transient service is collected on app exit; no unit is
installed or enabled persistently.

External launch supervision tracks the authenticated wrapper PID and kernel
start time across exec, together with existing capture/Steam process evidence.
PID reuse is exit, inaccessible process state is unavailable, and End retains
the launch lock while the actual game remains live. The same coordinator owns
capture, Replay, effective profiles, history, and cleanup for both entry points.

## Session lifecycle

1. Validate the saved game and installation, resolve inheritance, and check
   launch/runtime capabilities.
2. Prepare the private capture runtime for a supported launch, including when
   metrics are initially hidden. App-initiated native Steam forwarding verifies
   saved bridge options; direct Steam requests verify the running wrapper.
3. Spawn with literal arguments or publish a bounded one-shot Steam activation.
   Do not install a global Vulkan layer or edit Steam configuration silently.
4. Supervise the owned direct process or forwarded game identity in native code.
   Expose actual capture/encoder status; process existence is not proof of frames.
5. Permit show/hide of the already-prepared metrics renderer. No layer is injected
   into an unrelated or already-running game. Replay failures must not stall
   presentation or remove unrelated monitoring.
6. End session releases Redunar's session resources without killing the game.
   Retain the launch lock until the owned game exits. Natural exit records one
   history entry and releases resources; Quit uses the same cleanup ownership.

Missing capability must have an honest error/unavailable state. Automatic replay
is a request on supported launches, not a guarantee of successful recording.

## Metrics and history

One monitor worker caches sensor paths, reads hardware at 1 Hz, and publishes a
revisioned latest snapshot. Conservative process discovery is bounded and runs
at the slower cadence in PERFORMANCE. No UI callback queue or hardware scan is
permitted per frame. Missing sensors affect only their own values.

The Vulkan layer and launch-scoped OpenGL interposer batch monotonic frame
intervals over bounded local transport. The OpenGL path observes GLX/EGL swaps
and guarded x86_64 SDL2 dynamic-API presentation slots used when managed
runtimes such as .NET P/Invoke bypass normal ELF interposition. Redunar's
session-private library delegates SDL's table initialization to the exact
loaded SDL object and reapplies its swap and renderer hooks as part of that
initialization event. This remains correct when a game calls another SDL entry
before `SDL_Init` and does not depend on a startup delay or polling window. The
daemon records the producer API and accepts bounded Replay exports from Vulkan
or a capability-ready desktop OpenGL producer. OpenGL allocates a fixed GBM
pool, imports each allocation through `GL_EXT_memory_object_fd`, copies the
presented framebuffer asynchronously into an imported pixel-pack buffer, and
transfers one descriptor to the existing Vulkan Video conversion/encode path
only after its GL fence is ready. Slots remain unavailable until the daemon
returns a release acknowledgement. The producer binds its own private
per-process reply socket and drops Replay work instead of blocking presentation
when every slot is busy. The production path also requires complete
`GL_EXT_memory_object`/`GL_EXT_memory_object_fd` tokens and the matching memory
object functions; unsupported drivers and OpenGL ES/EGL Replay fail closed
before external-memory work. EGL metrics remain available. OpenGL is loaded
through a child-only `LD_PRELOAD` entry that
preserves inherited entries and is never installed as a global graphics
provider. Native Steam activation carries both private capture libraries through
the same bounded one-shot wrapper protocol; the OpenGL library is copied into
the session directory already shared with the Steam Linux runtime. When the
literal command/argv identifies a Proton interpreter, the wrapper removes
Redunar OpenGL preload entries before exec and keeps Vulkan capture and unrelated
preloads. Recognition preserves non-UTF-8 path bytes. Steam Runtime and
pressure-vessel alone do not identify Proton: native games using those runtimes
retain OpenGL capture. An empty preload override prevents an inherited Redunar
copy from being restored; unavailable activation still forwards the original
environment unchanged. Flatpak
Steam remains a separate unsupported sandbox boundary by owner decision.

Handle-specific GLX swap/context-destruction lookups retain their exact original
target when the interposer cannot forward to that same provider through
`RTLD_NEXT`, including privately loaded libraries. These calls bypass optional
capture instead of being dropped or redirected to another GLX provider. Matching
global targets retain the hooks. Failed lookup probes do not cache a missing
provider or leave an internal loader error on a successful application lookup.

The capture receiver validates export ordering per selected/provisional process
and translates each accepted wire sequence into a session-unique internal token.
`capture_session/replay_release.rs` retains the original reply endpoint and wire
sequence until safe completion. Helper-to-game handoff cannot reuse another
process's token or redirect its pending acknowledgements. Readiness follows the
confirmed producer's accepted source, including a smaller replacement surface.
Vulkan keeps its wire counter across device recreation. Each Vulkan/OpenGL
producer startup binds a fresh private reply endpoint without unlinking an
existing one. Device replacement, library reload, and PID reuse cannot redirect
an old buffer acknowledgement to a new incarnation. Backend prefixes also keep
two APIs in the same process separate; endpoint names stay bounded to 23 bytes.
Session preparation reserves the complete reply path within Linux's 107-byte
pathname socket limit before creating runtime files or arming capture.
The launch profile sends either a fixed rate or a distinct `variable` token to
the same private game capture path. Fixed capture retains deadline-based
timestamps; Variable mode stamps accepted presents with their monotonic game
presentation time and uses resolution-bounded admission. Vulkan and supported
OpenGL exports feed the existing hardware H.264 encoder. The stream advertises
a bounded rate ceiling while each encoded packet keeps its actual timestamp;
the ring and spool retain a bounded larger budget for this mode. The in-game
menu carries a distinct Variable value so an older fixed 120 FPS profile is
never mislabeled. Neither path observes desktop scanout.
The service validates observations and owns summaries. Completed sessions retain
bounded frame data and timeline observations through `session_history.rs`.
This is not an unlimited per-frame archive; older records may lack hardware data.

The private capture protocol is version 7. Both graphics producers send a
bounded GPU identity (PCI vendor and device/driver UUIDs), also attached to every
Replay source/export. OpenGL queries its current context's external-memory UUIDs;
Vulkan queries the actual physical device that created the presenting device.
The daemon matches encoder UUIDs before import and rejects frames whose identity
changes. Metrics/history use the capture vendor only when the cached hardware
snapshot contains one physical GPU of that vendor. Missing/ambiguous identity
omits GPU measurements. UUIDs are private, never persisted in session history
or printed in diagnostics. The app and both capture sidecars must be rebuilt
together; older wire versions are rejected. Profile/history formats are unchanged.

`crates/redunar-capture-vulkan/src/device_extensions.rs` owns bounded, complete
extension discovery for both the producer and Vulkan Video encoder. The native
Replay DTO distinguishes pending first frames from explicit source rejection;
the UI keeps recorder failures ahead of pending copy and enables saving only
from native recorder health.

The UI plots retained timestamps without smoothing or synthetic endpoints.
History keeps the existing v1 format and filename. Readers cap the file at
64 MiB and retain at most the newest 64 records; writes use unique private
temporary files, atomic replacement, and directory synchronization. Escaping
decodes the existing writer once so literal backslashes survive a round trip.
[History calculations](output/tauri-redunar/HISTORY-CALCULATIONS.md) owns axis,
selection, missing-data, and reconstruction rules. Do not equate a latest
frame interval with the reciprocal of an average FPS window.

## Overlay and menu are different surfaces

The Vulkan layer and OpenGL interposer render the metrics HUD, in-game Replay
menu, and Moment saved feedback. OpenGL derives the same metric presets and
layout selections from bounded frame history and daemon hardware telemetry. Its
four cached CPU RGBA surfaces use the shared overlay font and panel geometry;
context-local blended texture passes draw metrics, menu, cursor, and notice
revisions independently through GLX or EGL. The largest current viewport owns
presentation telemetry and Replay when a process has multiple GL contexts;
destroying that context releases its renderer, diagnostic readback, and Replay
bookkeeping. FPS only remains panel-free, and menu/notice
visibility does not depend on metrics visibility.
Metrics visibility is reversible without removing either prepared runtime.
Geometry/font details live in `redunar-capture-vulkan/src/overlay.rs`,
`redunar-capture-opengl/src/overlay.rs`, core font data, and the
[shader notes](crates/redunar-capture-vulkan/src/shaders/README.md).
The effective profile carries Grid, Ribbon, or Telemetry and one of eight
bounded palettes through direct-launch environment values, the versioned Steam
activation wire format, and the live overlay telemetry block. Branding
visibility follows those same launch and telemetry paths. Metric presets and
Custom metric bits remain separate, so changing structure or color never changes
which measurements are selected.

The active Tauri replay shortcut toggles the in-game Replay menu through the
daemon's replay control socket. The active Vulkan or OpenGL capture backend
renders the panel into the game's own presentation target, and the
hotkey helper grabs the mice and streams pointer events while it is open; no
desktop menu webview exists. If the session exposes readable keyboard devices
but no readable mouse event device, the helper still opens the in-game panel in
view-only mode and reports the missing pointer capability to Tauri. The app does
not expose a desktop preview of this game-rendered menu. The versioned menu
telemetry carries bounded display labels for the current menu chord and the
selected duration's save chord. Menu format clicks are daemon-owned preference
writes and update the active replay runtime before the next save.
The native tray exports StatusNotifierItem and DBusMenu objects through the GTK
GIO session-bus APIs already used by the host; it does not load AppIndicator.
One private connection owns the item, menu and worker while enabled. Disable and
shutdown close that connection and remove the registration. Watcher ownership
changes invalidate pending replies; host/item loss restores the window.
Registration requires this connection's exact unique name/path in the current
watcher's item list, a registered host and a successful registration reply.
Open/Quit actions are scheduled on the main thread; shutdown never waits for GTK
from the bus worker. Bounded registration checks only run for reachability.
Close to tray controls icon visibility immediately. Closing hides only when the
preference and usable tray registration allow reopening; otherwise it exits.
Loss of the tray host must not strand a hidden main window. Native hotkey
dispatch continues while the main webview is hidden; an empty binding set is
valid.

## Persistence and compatibility

The shared service starts the opt-in logger; one bounded writer owns its private
files, rotation, and runtime health. Tauri reads that health independently of
the saved restart-required preference and shuts the logger down after session,
media, shortcut, and monitor cleanup. Startup executable fingerprinting runs
once off the UI thread. Replay diagnostic aggregation lives in
`crates/redunar-daemon/src/replay_logging.rs`; NVIDIA capability events remain
allowlisted in `replay_nvidia_diagnostics.rs`. The UI never chooses a log path
or uploads diagnostics. [PERFORMANCE.md](PERFORMANCE.md#desktop-media-and-history)
owns the queue/file budgets; [REPLAY.md](REPLAY.md#capture-and-encoding) owns
recording diagnostics and [DESIGN.md](DESIGN.md) owns the settings behavior.

Local state normally lives below `$XDG_STATE_HOME/redunar` (otherwise
`~/.local/state/redunar`). Catalog and preference loaders validate versions,
lengths, paths, and values; writes use private atomic files. The catalog is
bounded to 1 MiB and 256 games. Store arguments as literal values; an editor
must not rewrite a representation it cannot preserve losslessly.

Catalog, application preferences, replay preferences, session history, artwork,
replay store, and temporary playback files have distinct owners. Loading a
missing executable must not delete its record. Changing the replay parent does
not silently migrate old recordings. Read only approved clip files through
native validation; cleanup removes owned temporary media, not original clips.

Format changes need old/fresh-state tests, migration behavior, and interruption
handling. Private diagnostics stay local. Do not add cloud sync, accounts, or
telemetry as an incidental implementation choice.

## Native privilege boundary

The Tauri application exposes no arbitrary privileged commands. Native commands
use narrow allowlists, preserve
user ownership, and never run the UI or whole service as root. Any future
privileged capability must snapshot the prior value, use one canonical writer,
detect external conflicts, and restore only values Redunar owns.

## Evidence and unresolved work

[ROADMAP.md](ROADMAP.md) lists current gaps; [TESTING.md](TESTING.md) separates
fixtures, native bridge checks, installed-runtime checks, and game acceptance.
The source map above is the current product contract and the only desktop
frontend ownership model for new product work.
