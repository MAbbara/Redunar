# Instant Replay contract

Current Tauri behavior, reviewed September 26, 2026. Recording and saving remain
local. This guide owns replay behavior; [DESIGN.md](DESIGN.md) owns presentation
and [PERFORMANCE.md](PERFORMANCE.md) owns resource budgets.

## Activation and settings

On a supported launch from Redunar or the configured native Steam
wrapper, replay is requested automatically. Recording settings configure the
buffer without a separate activation switch. Capability and runtime validation
still apply;
unsupported launches must not claim that a buffer exists.

Steam's wrapper can start a closed Redunar through the Linux user service manager.
The background app lives outside Steam's game reaper so it can stay open after
the game exits without holding Steam's Running status. If this startup capability
is unavailable, launch Redunar before pressing Play; the game still launches.
See [the bootstrap ownership contract](ARCHITECTURE.md#catalog-profiles-and-installation).

Proton Steam launches use the prepared Vulkan capture path without Redunar's
OpenGL interposer in the compatibility-tool startup chain. Native Linux Steam
games retain OpenGL capture, including through Steam Runtime. This does not add
OpenGL capture to Proton games; Vulkan/encoder capability gates still apply.
See [capture runtime ownership](ARCHITECTURE.md#metrics-and-history).

The prepared Vulkan or supported desktop-OpenGL runtime also permits
showing/hiding metrics during that game.
Hiding metrics does not dismantle capture or disable future visibility changes.
This does not allow attaching a runtime to an unrelated running game.

Recording mode and quality are configured before launch and locked until the
owned game closes. **60 FPS** keeps fixed-cadence capture; **Variable FPS**
timestamps accepted game presents. Existing 30/120 FPS profiles remain readable,
with fixed 120 retaining its previous 1080p surface limit. Variable FPS is
bounded by the encoder's macroblock throughput: at most 240 captures/second
at 1080p and 144 at 2560×1440. GPU backpressure may drop frames. The source is
the launched game's Vulkan or supported desktop OpenGL presentation hook, not
the whole desktop or compositor scanout, and no picker appears. The same
Efficient, Balanced, and High quality presets apply to either mode before launch.
The output container remains selectable between MKV and
MP4 from Global settings or the in-game Replay menu; changes apply to future
saves without rebuilding the active buffer. The menu labels Variable FPS as
Variable, rather than showing the encoder's maximum capture rate. If its state
cannot reach the game, shortcut activation reports a failure without taking
mouse control. The default is 60 FPS / Balanced.
Supported save lengths are 15/30 seconds and 1/2/3/5/10/15 minutes. The spool retains the
bounded 15-minute horizon; choosing a shorter manual save length selects a suffix,
not a smaller rolling horizon. Fixed 120 FPS is gated by display/game-surface
limits of 1080p or lower and actual encoder capability. Variable mode retains
bounded extra producer buffers and ring/spool space so faster bursts do not
shorten the requested history solely through the packet count cap.

When a game reduces presentation rate or mutes its output while unfocused,
Replay preserves those sparse frames and silence. The recorder cannot recover
content the game did not produce. Use a save shortcut or the in-game menu to
keep the game focused; background rendering/audio settings remain an explicit
game/compositor choice. Fixed capture reanchors the first resumed frame after
a missed interval instead of stamping it with an expired deadline.

Global settings also owns replay folder, initial save duration, menu dismissal,
and shortcuts. Changing the folder parent keeps existing files in their old
location. No user-facing total storage quota exists; saves respect the filesystem
reserve and must not erase other recordings to make room.

## Shortcuts and menus

Fresh preferences assign **Alt+Shift+Z** to opening the replay menu. All direct
save bindings start empty; the menu initially selects 30 seconds. Existing saved
bindings, including cleared shortcuts, are preserved. Versions 1–3 retain their
historical Shift+Tab menu binding when migrating. These are Redunar defaults,
not desktop bindings or a guarantee against another application's hotkey clash.

The menu and up to eight save bindings are optional. Save-only, menu-only, and
completely empty configurations persist without restoring defaults on restart.
Clearing every shortcut stops keyboard activation; automatic buffering and the
app's Save replay action remain available. Validate assigned chords for duplicates.

Tauri uses the same-user `redunar-hotkey-helper` with bounded evdev reads. It does
not request Polkit authorization or edit desktop-global shortcuts. The RPM's
logind `uaccess` rule supplies access where supported; unavailable permissions
must be reported honestly. Native dispatch works with the main webview hidden.
Background startup also locates the matching helper beside a local build or in
an unpacked package's sibling `libexec` directory; it does not need inherited
game environment overrides. This lookup does not grant input-device access.

Keyboard discovery retries once per second while the helper is running, including
when no keyboard is readable at startup. A reconnect replaces the old input
handle even if Linux reuses its event path. Device loss clears that device's
held-key state and closes an open Replay menu to release pointer control.
When no readable keyboard remains, activation reports Unavailable and waits for
access to return; it becomes Active automatically after a successful open.
A mouse dongle may expose a valid keyboard interface, so Active means at least
one readable keyboard interface, not proof that every attached keyboard is
monitored. Saved shortcuts and unsaved UI drafts are unchanged by rediscovery.

The shortcut opens the Replay menu inside the captured game: the active Vulkan
or OpenGL capture backend renders the panel into the game's own presentation
target, so compositor stacking rules and fullscreen focus games cannot hide it
behind the game window. There is no
desktop menu window. The helper owns the pointer: while the menu is open it
grabs every mouse through the kernel so the game receives no pointer input, and
streams coalesced `MENU MOVE`/`MENU BUTTON` events over the replay control
socket. Each opening refreshes the readable mouse devices, replacing handles
whose input nodes disappeared (for example after a mouse reconnects). After
grabbing all mice, the helper discards queued gameplay input
before forwarding menu events, so an earlier click cannot dismiss or activate
the newly opened menu. Queue cleanup is bounded; a read failure or queue that
does not become empty releases all mice and uses view-only mode. The daemon
hit-tests menu events and answers `OK GRAB`/`OK RELEASE`. Escape,
any unrelated key (so Alt+Tab is never stranded), the helper's inactivity
timeout, and the daemon's command watchdog all release the mice and close the
menu. Capture teardown also closes the menu, and a subsequent pointer ping
receives RELEASE if the render target disappears. Some login sessions grant
keyboard access without granting raw mouse
access. In that case the menu still renders in a view-only fallback, the save
shortcuts remain active, and the shortcuts panel reports that pointer control
is unavailable; a transient kernel mouse-grab failure uses the same fallback
instead of cancelling the menu. Composite keyboard interfaces are evaluated in
one bounded chord window so modifier and function-key reader scheduling cannot
drop a configured chord. View-only mode sends menu heartbeats but does not
forward input from unowned mice. Pressing the assigned menu chord again or any
unrelated key closes it. The menu renders even
when the metrics overlay is hidden. Its bounded game-rendered surface and control
outlines use rounded corners, and measured labels are centered within their
cells so scaling cannot push shortcut or status text across a divider. The menu
follows the approved modern reference look: quiet letter-spaced labels above
bright values, thin dividers, a dark-red selected duration cell with a red
underline, and a red-outlined save button on a near-black rounded panel. The menu
shows the current menu chord and the selected duration's direct-save chord from
live persisted preferences; cleared bindings show as unassigned. Its format
picker persists MKV or MP4 and updates the active save runtime. A replay menu
requires a running captured session, and without one the helper reports the rejection and the shortcuts
panel shows it. The app does not expose a desktop preview because the production
menu is rendered by the capture backend inside the captured game.

A completed save drives the bottom-left **Moment saved** pill in the game,
including duration and Local library. It is independent of metrics visibility.
Do not show success on key press alone: completion must come from the native
save lifecycle. Buffer readiness, saving, unavailable, and failure are distinct.

## Capture and encoding

The production path uses private game-owned graphics exports, not KMS or desktop
screen scraping. Vulkan supports its implemented 8-bit and packed 10-bit
swapchain paths. Vulkan Replay requires the game's requested instance API and
physical device to support Vulkan 1.1 or newer; Vulkan 1.0 stays metrics-only.
Redunar preserves the game's API request. On the validated AMD/RADV host, desktop OpenGL uses a fixed
six-slot pool of linear GBM RGBA8 buffers per bounded context, imported through
`GL_EXT_memory_object_fd`;
fence-ready descriptors pass to the same daemon-owned GPU conversion and
hardware H.264 worker. EGL/OpenGL ES Replay remains unsupported. Every route
still depends on format, dimensions, device, queue, driver, and encoder gates.
Source shapes are bounded to 8,192 pixels on either axis and 8,294,400 total
pixels (the existing 3840×2160 budget), including 5120×1440 ultrawide and
2160×3840 portrait sources. These are software admission limits, not a hardware
support guarantee. The selected encoder must still accept the exact coded
extent and H.264 level. Fixed 120 FPS retains its 1080p surface limit; Variable
FPS uses the resolution-dependent macroblock ceiling, including 72 FPS at
5120×1440 and 64 FPS at 3840×2160. Larger pixel counts remain unavailable;
recording does not downscale the game. A resolution rejection refers to game
resolution and recording rate, independently of fullscreen/windowed mode.
With Beta access enabled at startup, one identifiable NVIDIA render GPU may
attempt the same Vulkan Video H.264 path, including alongside known Intel/AMD
render GPUs. The driver must expose the required Vulkan Video encode,
external-memory import, and queue capabilities. Vulkan and supported desktop
OpenGL producers attach the current game's device/driver UUIDs to each source
and export. The encoder must match both identities before importing; a missing
match never selects another GPU. OpenGL additionally needs one physical render
GPU for its context's vendor to select the GBM allocator. Unknown topology and
multiple NVIDIA GPUs remain withheld. This is an unverified beta path, not an
NVIDIA recording guarantee.
Capture export and encoder discovery share a complete device-extension query:
up to 1,024 entries and four count/fill attempts when Vulkan returns
`VK_INCOMPLETE`. A larger or persistently changing list fails closed; a partial
list never establishes support. Required extensions and device identity still
gate encoder creation.
When Debug log was enabled before launch, NVIDIA encoder startup reports
allowlisted stages, readiness, bounded capability rejection reasons, exact known
missing extension names, source identity availability/match, and numeric Vulkan
results. Device UUIDs, PCI addresses,
paths, and arbitrary driver strings are excluded. NVML readiness and optional
sensor availability are logged on changes. App startup records the version,
effective Beta setting, and a bounded asynchronous fingerprint of the running
executable; failure to read the fingerprint is reported as unavailable.

The local file includes wall-clock milliseconds and monotonic elapsed time.
Recorder events cover first source/encoded/audio data, resize, recovery, save
commit, and shutdown. Five-second aggregates report Variable FPS source intervals,
fixed-cadence source timestamp gaps,
submission latency/failures, in-flight work, spool drops/slowness, and audio
packet gaps. Source gaps are observations, not claims about window focus;
received-minus-encoded counts are not treated as a dropped-frame measurement.
The general log may still include game names and session details. Legacy
messages with path-like tokens have their remaining text suppressed; new
hardware events use typed, non-identifying values. Logs stay local until the
owner explicitly shares them. See [the first NVIDIA test](TESTING.md#nvidia-beta-first-hardware-test)
and [logging bounds](PERFORMANCE.md#desktop-media-and-history).

Backpressure drops replay work rather than waiting for an encoder on the game's
presentation path. Resize starts a fresh codec epoch: completed old-generation
copies are discarded locally, already-exported buffers remain release-gated,
and the replacement source is announced only after its bounded pool exists.
Context destruction closes producer ownership while transferred DMA-BUF
duplicates remain valid in the daemon. Audio or recorder
failure must not stall the game. Recording has no software-video fallback;
FFmpeg's role in playback/export below is separate from live capture.
Producer-release sends are nonblocking. Failed safe acknowledgements are retained
for retry on later pump turns, including turns with no new frame. Audio operational messages report
only the allowlisted backend and transition, without output-device names.
Exports are ordered within each selected/provisional process. The receiver
assigns unique internal tokens and keeps the original process reply endpoint
and wire sequence until encoder completion. A helper-to-game handoff therefore
accepts lower or overlapping game sequence numbers without releasing unfinished
helper inputs. Duplicate in-flight exports never trigger an early acknowledgement.
Only increasing exports count toward producer confirmation, and readiness follows
the accepted source even when its surface is smaller than the helper's.
Accepted device-recreation handshakes reset confirmation and local ordering while
preserving outstanding internal tokens. Vulkan's export counter does not reset
when its last device is destroyed. Each Vulkan/OpenGL startup binds a fresh
private reply endpoint, preserving old ACK ownership through library reload,
PID reuse, and API changes. Copied-frame diagnostics follow only the
confirmed process after handoff and reset their sequence watermark for that owner.
An overlong runtime directory fails session preparation with a clear error before
capture is armed; validation reserves the entire producer reply filename.
At the shared reply-route cap, capture intake pauses before receiving another FD;
capacity recovery resumes intake without losing a producer acknowledgement.
Debug logs record pause/resume transitions without token or endpoint values.
An active producer's source rejection clears the Replay candidate and recording
epoch without ending metrics or the overlay. A new eligible announcement and
export can re-arm after the bounded recovery delay. Rejection survives rapid
replacement announcements; queued, unsubmitted exports are released separately
from GPU inputs. Failed encoder teardown stays failed through repeated cleanup,
withholds unfinished buffer acknowledgements, and blocks re-arming. Vulkan
retains that failed encoder's native resources until process exit; restart
Redunar before another attempt. Finite fence waits do not make a stuck driver
call interruptible.
Vulkan presentation waits have a separate lifetime from copy fences and DMA-BUF
leases. Queue/device idle alone does not authorize destroying their semaphores.
The producer retains two retired source generations plus one paused generation
until the independent completion proofs exist or the game destroys its device.
If old swapchain images are never reacquired with a signaled acquisition fence,
repeated replacements can exhaust that budget and make Replay unavailable for
the session. This conservative limit needs hardware evidence and future
presentation-completion support before claiming unrestricted resize recovery.
If the daemon exits, the game continues presenting; telemetry batches reset
after failed sends and exported OpenGL slots remain bounded while release
messages are unavailable. A restarted daemon starts a new capture session,
so Replay requires a new game launch from Redunar or configured native Steam;
the old game process cannot attach to a different session. Startup sweeps stale private capture
socket directories.

When a process presents from multiple OpenGL contexts, the largest current
viewport owns metrics and Replay. Context destruction clears that choice so a
replacement context can take over immediately. OpenGL renderer and diagnostic
readback bookkeeping also relinquish the destroyed context's bounded slots.

A save assembles available retained history, including the active spool tail,
into a private local MKV or MP4. It requires a healthy populated buffer and
preserves a 512 MiB filesystem reserve. Store markers, validated names, atomic
commit, no-overwrite behavior, and owned-file deletion prevent arbitrary access.
An audio track is included only when its packets overlap the saved video window;
out-of-window audio must not produce an empty audio track.
Save success is reported only after native completion and inventory revision.
MP4 frame durations extend to the next video presentation timestamp even when
that frame starts a new keyframe, time-limited, or byte-limited fragment. Only
the terminal sample uses its declared packet duration. This preserves the last
available picture through a source pause without fabricating new frames.
Save preparation queues an ordered spool snapshot; a full queue rejects the
request for retry without stopping healthy recording. Assembly streams pinned
segments into the private output, and shutdown cancels pending assembly. See
PERFORMANCE for the snapshot deadline and payload bounds.
Each committed clip name is queued beside the completed-save revision so the
game-session coordinator can attribute the clip to the recording game. The
attribution lands in the private bounded ledger
`replay-clip-games-v1.tsv` in the state directory and is labeling data only:
ledger failures never affect the save, and clips without a ledger line are
resolved later by matching the commit time encoded in the clip name against
exactly one recorded session window.

## Audio behavior

Replay records the default output monitor: the same mixed audio the user hears.
It does not depend on game-process ownership. Microphone capture is enabled by
default using the system-selected input; it can be switched off in Replay
settings between game sessions. The native service rejects microphone changes
through launch, recording and teardown; a failed write restores the saved UI
value and leaves the control available to retry. Version 1–6 preferences migrate
with microphone capture off. When enabled it is captured on a separate bounded
timeline and mixed into one Opus track during save, so VLC and other players
receive system sound and microphone speech in their selected audio stream. Device selection is best effort and a missing microphone never
stops video or system-output audio. System-output capture deliberately includes
other applications that play through the same output, so it must be described as
system-output audio rather than isolated game audio.

`crates/redunar-capture-audio/src/source.rs` prefers the native PipeWire route:
when the PipeWire registry exposes a default output node, Redunar records it
directly with `pw-cat`, explicitly requesting that sink's monitor ports.
Recording through pipewire-pulse's compatibility layer
instead was observed on 2026-09-24 to disturb a game's own Pulse client
(Stardew's music went silent while a Pulse monitor recorder was attached during
a Redunar session and returned once the recorder was removed), so the Pulse
monitor API now serves as the fallback for hosts without a usable native
PipeWire route; it still works with a native PulseAudio server and with
PipeWire's Pulse server. Both paths produce fixed 48 kHz
stereo PCM, bounded 20 ms Opus packets, and timestamped muxing. Microphone
input uses the same packet and clock contract, with its own bounded timeline. The
worker puts encoded audio into bounded timelines, so hardware video encoding and
the save-time encoder drain cannot stall audio ingestion. A save reads the audio
snapshot for the selected video window after draining video output, including
the clip tail. The assembler then mixes outside the runtime and audio locks,
with one decoded frame per input and cancellation checks on each output frame.
Sample offsets align overlapping frames. Microphone speech survives missing
system packets, and a microphone-only clip uses that input as its sole audio
track. Gaps in both inputs remain timestamp gaps; decode/encode failures fail
the save instead of silently producing separate tracks. Coincident PCM peaks
saturate safely; a silent input does not halve the other input's volume.
Both audio timelines are cleared when the video epoch resets or Replay stops.
A resize cancels an unfinished save before resetting its audio epoch; already
committed clips are retained.
Audio activity tracks successful packet ingestion time independently of retained
packet count, so a full rolling buffer continues to report actual progress.
Because the PCM helpers do not provide timestamps, capture advances in 20 ms
steps while samples arrive steadily. Audio starts on the same monotonic clock
as game-frame timestamps. If the PCM timeline drifts more than 250 ms from
that clock, the worker discards the helper's queued samples and reconnects;
it never assigns current timestamps to stale sound.
The MP4 muxer starts a new audio fragment at each gap so that the time jump is
preserved in the saved clip.
One separate worker rechecks the default route every two seconds after each
completed check, with one-second deadlines and kill/reap cleanup for each
discovery command. The audio-reading worker never waits for route discovery.
It begins a retry after 250 ms following a
route change or capture failure, and treats three seconds without samples as a
stalled transport that must be restarted. When both backends are available, a
failed or stalled recorder is retried through the other backend instead of
repeatedly selecting the same broken route. Older PulseAudio clients without
`pactl get-default-sink` use the long-standing `pactl info` result.
The microphone has its own route watcher and the same retry/stall ownership
rules. Default input discovery resolves `wpctl` metadata or `pactl` before
selecting a PipeWire source, so multiple physical inputs do not make the
configured default ambiguous. Explicit selected-input names remain supported
by the native API. Older PulseAudio clients use `pactl info` in the C locale
when `get-default-source` is unavailable. Failed helpers are killed and reaped
before any retry; discovery does not block active PCM reads.

Redunar therefore requires either a working PulseAudio-compatible server with
`pactl` and `parec`, or a working PipeWire server with `pw-dump`, `wpctl`, and
`pw-cat`. PipeWire itself is not mandatory. A pure ALSA setup has no standard
monitor for already-mixed playback and is unsupported unless the user routes
output through PulseAudio or PipeWire. Missing audio still leaves video capture
running and must be reported honestly.

## Clip browsing, playback, and export

Native inventory supplies names, sizes, timestamps, and available game/duration
metadata. The game name follows the persisted attribution ledger first and the
unique-session window fallback otherwise; ambiguous or missing evidence shows
"Game not recorded" rather than a guess, and a trimmed export inherits its
source clip's attribution. Inventory requests first flush names committed by the
save worker into the attribution ledger, so the completion refresh includes the
recording game. Read thumbnail/metadata files only through validated local paths.
Selection preserves the clip rail, search, scroll, and focus. One preparation
runs while a newer pending selection replaces older requests; stale completions
cannot replace the selected player. A newer selection or navigation away cancels
the active native preparation. Expensive media work runs off the UI thread.

Playback lazily starts a loopback-only HTTP listener with random per-selection
256-bit tokens. It serves an already-opened validated file through GET/HEAD and
bounded ranges, checks Host and supplied Origin, and accepts no arbitrary path.
A selection change invalidates old streams; shutdown joins readers and cleans up.

Native preparation uses FFmpeg to make a private fast-start MP4. H.264 is copied;
optional Opus audio is converted to AAC for WebKitGTK. This also normalizes MP4
inputs. A bounded temporary copy, 30-second preparation deadline, and useful
conversion/storage errors protect the player. Temporary allocation can fall
back from a constrained default temporary filesystem to a private `/var/tmp`
directory. Source recordings are unchanged; prepared copies have private access
and are removed when replaced or on normal shutdown.
Startup also sweeps leftovers a crash or kill cannot clean itself: private
`/var/tmp/redunar-player-*` copies older than one hour and stale
`$XDG_RUNTIME_DIR/redunar/capture-*` session directories (including in-game
reply sockets) owned by this user. Audio-capture and FFmpeg children get
`PR_SET_PDEATHSIG`, so a killed Redunar cannot orphan a live recorder.

Playback waits for a decoded frame, not just metadata. The media-load deadline
is 15 seconds after preparation, with Retry on failure. The player follows the
clip aspect ratio without cropping. Eight filmstrip previews use one temporary
muted decoder; timeouts or missing previews must not disable valid playback.
See PERFORMANCE for worker/decoder bounds.

Export re-encodes the selected range for accurate boundaries, reports progress,
supports cancellation, and commits a separate clip. The original is never deleted
or archived as part of trimming. FFmpeg playback/export capabilities must be
validated separately from the live hardware encoder. Native checks resolve the
installed executables from PATH, check required encoders on demand, and diagnose
clip-specific missing codec/format support. Successful encoder checks are cached
for up to a minute; a missing capability is checked again on retry so installing
codecs does not require restarting the app. No app path installs or swaps packages.

The RPM requires the ffmpeg/ffprobe executable paths and a complete codec
capability that either RPM Fusion's full libraries or its standalone freeworld
library supplies. It also requires WebKit's GStreamer libav decoder plugin.
Existing compatible providers satisfy the requirements; missing requirements
are handled by the package manager. See [packaging](packaging/README.md).
Other distributions need their own packaging metadata; runtime checks do not
identify a distribution or require an RPM package name.

## Validation and history

Use [TESTING.md](TESTING.md) for deterministic media tests, native WebKit probes,
installed-component verification, and owner-controlled game acceptance. Fixture
encode success does not prove real-game frame pacing or cross-distribution
playback. Preserve original files during diagnostics; copies belong in isolated
state. Retired toggles and obsolete readiness behavior must not be reintroduced.

## Recording filenames, favorites and capacity

The replay browser displays actual filenames. Edit filename exposes only the
stem and retains the original extension. The native service/store accepts only
owned inventory entries, uses an atomic no-overwrite rename and preserves game
attribution and favorite state. Active exports serialize against renaming;
playback retains its open descriptor. Collisions, traversal, hidden/control names,
symlinks, hardlinks and replaced renamed files are rejected.

A bounded private `.redunar-clip-labels-v1` ledger in the owned replay directory
records device/inode, original generated filename and favorite state. Destination
identity is durably staged before rename, so interrupted operations leave either
name discoverable. Failed precommit metadata writes leave the recording untouched.
The inventory still excludes arbitrary videos; only explicitly registered renamed
identities supplement the original generated-name reader. Old files need no rewrite.
This ledger is an ownership reader and must remain until all renamed recordings
have been migrated; older Redunar versions will not list renamed files.

The Replay drive panel distinguishes total filesystem usage/free capacity from
recordings' size. Missing capacity omits the meter; low space suggests explicit
review in the clip folder, with no automatic cleanup added. Capacity uses the
nearest existing destination parent before the first recording creates the store.
Use default folder restores Videos/Redunar Replays (state-directory fallback when
HOME is unavailable), and never moves existing recordings.

The folder picker selects a parent only; service validation still owns the fixed
Redunar Replays child. Input enumeration uses bounded PipeWire/Pulse discovery,
excludes output monitors, reports unavailable inputs and preserves an unplugged
saved selection. Input selection applies to the next launch and remains locked
through launch, recording and teardown, as described in Audio behavior.
