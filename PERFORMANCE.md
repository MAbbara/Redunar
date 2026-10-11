# Runtime performance contract

Current engineering budgets, reviewed September 23, 2026. Redunar measures games;
its own work must not meaningfully disturb them. Budgets are acceptance targets,
not a promise of performance improvement or a report that every path passes.

## Budgets

For the cached monitor while no active frame capture is running:

- Average monitoring CPU: below 0.5% of one CPU core.
- Hardware sample latency: below 2 ms at p95 and 5 ms at p99.
- RSS growth: no more than 1 MiB during ten minutes of idle monitoring.
- Bounded UI work: never accumulate a callback or queued snapshot per sample.

During active frame capture, Redunar's processing should stay below 1% of one
core; the metrics overlay shares this budget. Measure replay workers, audio,
helper CPU, GPU encode cost, and game frame pacing explicitly. Do not claim the
whole recorder meets this target from a monitor-only or bookkeeping benchmark.
Record process scope, host, driver, build, workload, duration, and method.

## Collection and presentation rules

- Discover sensors once and sample cached hardware paths at 1 Hz. Use counter
  deltas for CPU load; represent missing/reset counters explicitly.
- Conservative game-process scans run no more than every five seconds unless a
  known lifecycle event requires action. Never spawn a process per sample.
- Publish an overwrite-only latest snapshot, keep histories bounded, and batch
  frame telemetry across process boundaries. Do not redraw unchanged values.
- Keep Vulkan telemetry nonblocking. Do not discover hardware, spawn subprocesses,
  wait for UI/service work, or format routine logs in the presentation path.
- Keep rendering geometry, text, and GPU resources bounded. Reuse resources and
  skip telemetry/overlay/replay work under contention instead of stalling a game.
- Use native supervision and shortcut workers independent of visible webview
  polling. Stop and join owned workers promptly on shutdown.
- Keep catalog/artwork/media discovery outside monitor ticks and frame callbacks.
  Bound file sizes, records, parsed arguments, worker counts, and queues.

## Replay bounds

Vulkan device-extension discovery allocates at most 1,024 fixed 260-byte records
per attempt, with four count/fill attempts for an unstable list. Queries run at
device preparation or encoder startup, outside presentation and UI polling.
See [capture and encoding](REPLAY.md#capture-and-encoding) for capability policy.

The current capture/encode pipeline has four conversion/encode slots. Vulkan
uses a fifth export handoff context. OpenGL uses six release-gated slots per
context: one handoff for encoder completion and a second for SDL swap dispatch.
OpenGL keeps at most four context pools; actual 4K RGBA allocations therefore
remain below 800 MiB per process, while ordinary 1080p contexts use about
48 MiB each. Do not reduce either producer pool without checking fence,
presentation-hook, and acknowledgement progress. Encoding has no
host-pixel/software-video fallback.

Ultrawide/portrait admission keeps the same 8,294,400-pixel frame budget while
allowing up to 8,192 pixels per axis. Shared protocol, producer and encoder
validation enforce this before frame allocation/import. OpenGL retains its
33,177,600-byte per-slot limit, including row padding; wider padded allocations
that exceed it remain unavailable. Pool, pipeline, spool and packet budgets
are unchanged. Actual encoder coded-extent and H.264 level checks still apply.

Source-size generations share the four-pool process cap. A resize discards
fence-complete local copies from the old generation and waits for daemon-owned
exports before deleting that generation. Explicit GLX, EGL, and SDL context
teardown removes every pool and renderer/readback bookkeeping entry owned by
the destroyed context, preventing repeated context replacement from consuming
the cap. The producer selects the largest current viewport as its one active
presentation source, so an auxiliary window does not double its frame count.
Diagnostic OpenGL readback tracks at most eight contexts separately from the
four production pools. The daemon queues at most eight transferred exports and
tracks at most eight provisional producer paths. Source rejection also bounds unsubmitted release
retry tokens and all outstanding reply routes to 64, with at most eight additional
invalidated frames held for release only. The same route cap includes queued and
GPU-owned inputs; it is not 64 per process. At the cap the receiver pauses intake
before receiving another FD, leaving bounded kernel/producer queues to supply
backpressure until a route is released. Shutdown still checks its stop flag.
The worker retries safe releases
separately from unfinished GPU inputs.
Vulkan retirement retains two old routes plus one paused route while copy,
presentation, or external ownership is unproven; an invariant-violation slot
preserves ownership without aborting the game. See the
[presentation lifetime limit](REPLAY.md#capture-and-encoding).
Frame telemetry batches at most 64 intervals or 250 ms. Presentation takes only try-locks, polls fences
without waiting, and drops Replay work when all six slots or four pools are
busy. Repeated malformed-source logging is limited to one entry per five
seconds and omits raw transport errors that could contain private paths.

Game GPU identity adds 40 fixed wire bytes to source/copy/export metadata without
increasing the maximum datagram bound. Vulkan queries identity at device creation;
OpenGL caches at most 16 context identities and clears them at context teardown.
GBM topology selection runs once when creating the process allocator. Repeated
presents do not rescan DRM or query device UUIDs; unchanged identity emits no
additional datagram. These bounds do not establish NVIDIA frame-pacing performance.

Encoded spool commands use a four-item nonblocking queue. Existing packet bounds
limit worst-case queued payload; complete segments are bounded to 32 MiB/four
seconds and 512 entries. The spool retains a 15-minute horizon, with bitrate-based
disk limits rather than RAM proportional to the requested duration. Save work
includes the active tail and preserves the store's 512 MiB filesystem reserve.
Snapshot requests use the bounded command queue without waiting on the runtime
thread. The spool worker seals and pins files in command order; an assembler
waits up to five seconds for that response and supports cancellation. Status
reads use published counters rather than the filesystem index lock.

Saving indexes packet offsets within the existing packet/byte/duration limits;
it does not reload the selected clip into an encoded ring. MKV pulls one packet
at a time; MP4 retains at most an 8 MiB video fragment plus one lookahead packet.
Individual packets remain capped at 8 MiB; packet conversion can temporarily
duplicate that one payload. The metadata index follows the bounded frame count,
and each enabled audio input retains a separate 32 MiB bound. These are implementation bounds,
not measured whole-process RSS. Clip muxing does not hold the inventory lock;
ownership and current inventory are revalidated before atomic commit.
The authoritative constants live in `replay_spool.rs`, `replay_encoder.rs`, and
`replay_store.rs` under `crates/redunar-daemon/src/`.

Audio uses fixed 20 ms Opus frames and one long-lived capture process, with
retention capped at 45,000 packets/32 MiB per input (up to 64 MiB for system
output plus microphone). Opus encoding uses one fixed scratch frame and
allocates each retained payload to its encoded length, rather than retaining a
maximum-size allocation for every packet. Saving copies encoded packets only for the selected
video window; those snapshots have the same per-input limits. The assembler
keeps at most one decoded 20 ms PCM frame per input and one output accumulator,
then retains at most 45,000 packets/32 MiB of mixed output. PCM storage and
lookup work do not grow with the retained history; mixing visits selected
packets in timestamp order and checks cancellation each frame. These separate
bounds include additional save-time storage and are not a whole-process RSS
measurement. Audio discovery and retry logging are bounded; audio failure must
not stop video. See [REPLAY.md](REPLAY.md#audio-behavior) for capture and gap
semantics.

Fixed 120 FPS is subject to the implemented display/surface limits and hardware
validation. A capability check or short encode probe is not sustained frame-pacing
proof. Measure drops, queue pressure, disk latency, memory, GPU use, and output
quality through resize/reset and long sessions.
Variable FPS admits at most 240 selected game presents per second at 1080p and
144 at 2560×1440 under the H.264 macroblock ceiling, with a two-frame burst
allowance. It uses eight bounded Vulkan producer buffers, while fixed-rate
capture keeps five; the encoder remains four slots. Ring and spool byte budgets
double only in Variable mode. A source or encoder backlog drops Replay work
without pausing game presentation.
The same ceiling admits at most 72 FPS at 5120×1440 and 64 FPS at 3840×2160;
Variable mode does not require a minimum of 120 FPS at higher resolutions.

## Desktop media and history

Debug logging uses one writer thread and a nonblocking 256-record queue with
at most 2,048 message bytes per record. Only the writer performs file writes
and rotation; two private files are limited to 4 MiB each. Queue saturation
drops messages and reports a count. Repeated consecutive messages within five
seconds are summarized. Recording diagnostics aggregate every five seconds
on daemon workers; no file IO or routine log formatting runs in game presents.
Shutdown allows 500 ms for draining before detaching a filesystem-blocked
writer, retaining its owned resources until it returns or the process exits.
Logger failure is exposed in Settings. These are bounds, not a measured
whole-app performance claim.

- Clip selection keeps the rail mounted and preserves scroll/focus. One frontend
  preparation is in flight, with only the newest pending selection retained.
  Thumbnail workers remain bounded; completions update individual images.
  Basic metadata and thumbnails precede optional selected-clip packet counting.
  Native metadata caches at most 64 file identities, including inode, size, and
  modification/change timestamps. Inventory presentation caches expire after
  one second and invalidate on directory/attribution/history changes; the store
  and opened clip descriptors are still validated.
- Playback preparation runs off the UI thread, with one private cached MP4,
  bounded output, cancellation/cleanup, and a 30-second native deadline.
- The selected player waits for a decoded frame with a 15-second media deadline.
  A failed preparation or decode offers Retry; it must not spin indefinitely.
- Eight filmstrip samples use one temporary decoder and a fixed 1280×90 RGBA
  canvas (450 KiB). Wait while playing/hidden, cancel on selection/navigation,
  and release the decoder after completion. Each media load/seek has a
  2.5-second timeout. Do not regenerate thumbnails per frame.
- History maps retained samples once and uses bounded/logarithmic lookup for
  cursor inspection. Do not rebuild all traces on every pointer movement or
  smooth away measurements to reduce rendering work. See the
  [calculation contract](output/tauri-redunar/HISTORY-CALCULATIONS.md).

## Measurement procedure

September 23, 2026 AMD Radeon RX 6800 XT, Mesa 26.2.3
radeonsi/RADV, Fedora 44 kernel 7.2.7, working tree based on `659b519`:

| Synthetic 1080p route | Duration | Presented FPS | Replay packets | Receiver CPU |
| --- | ---: | ---: | ---: | ---: |
| Uninstrumented GLX, 7,200 frames | 61.25 s | ~117.6 from elapsed time | — | no receiver |
| Metrics-only GLX, 7,199 frames | 58.74 s | 122.80 | — | 0 sampled ticks |
| GLX Replay at 30 FPS, 7,199 frames | 60.97 s | 118.29 | 1,821 | 187 ticks |
| SDL OpenGL Replay at 60 FPS, 7,199 frames | 59.87 s | 120.37 | 3,581 | 207 ticks |

All captured routes had zero transport drops/rejects. The Replay probes ended
with 5 FDs and 2 threads, down from 8/3 before recording. A separate 640x360
held-release run kept all six export slots owned by the receiver: 318 Replay
work items dropped while 599 frames presented, with no transport failures.
The workload is a simple swap-loop, and the routes are not identical games;
these numbers establish bounded behavior, not a game FPS prediction or a
per-core CPU budget pass. Three consecutive GLX/SDL/fullscreen production
matrices also passed with bounded FD/thread teardown. Mesa's system-wide free
VRAM changed from 15,341 MiB to 15,338 MiB across those runs; that coarse
reading cannot attribute GPU memory to one process. GPU utilization and
long-session VRAM still need a separate owner-controlled observation.

Run from the repository root using a release build. For cached-monitor timing:

```sh
cargo run --release --offline -p redunar-daemon --example monitor_profile -- 60
```

That read-only probe measures the monitor, not the whole app. For a production
Tauri process, measure the current PID with `pidstat`/`ps`, include native helper
processes, and take before/after RSS. Record at least a ten-minute idle interval
for the growth budget. UI, Vulkan, recording, and codec checks are separate.

For bounded Replay bookkeeping and temporary-store throughput:

```sh
cargo run --release --offline -p redunar-daemon --example replay_profile
cargo run --release --offline -p redunar-daemon --example replay_store_profile
```

The first pushes ten simulated minutes of fixed-size packets through the Replay
ring. The second writes and removes one synthetic 16 MiB container under the
temporary directory. They can expose regressions in those components, but they
do not measure capture, hardware encode, decoded playback, game frame pacing, or
sustained recording. Record the build, host, storage, and observed values before
comparing runs. See the remaining retained diagnostics in
[TESTING.md](TESTING.md#retained-engineering-diagnostics).

Use matched game runs with the same scene, display mode, graphics settings, and
other overlays. Report duration, frame-time distribution/lows, variance, CPU/GPU
cost, dropped/rejected work, and cleanup. A shorter or noisier run must not be
presented as proof of an improvement. Owner-controlled procedures live in
[REAL-GAME-TESTING.md](REAL-GAME-TESTING.md).

Fixtures for counter resets, missing sensors, latest-value delivery, bounded
history/queues, cancellation, and prompt shutdown belong in ordinary tests.
Real GPU/game probes are explicit diagnostics, not a prerequisite for editing
unrelated UI copy or documentation. See [TESTING.md](TESTING.md).

## Historical evidence

Do not use stale implementation descriptions or a pass on an older build as
current acceptance.

The native Steam request listener has one worker and one fixed 16-byte request,
with same-user credentials and no game argv. Idle accepts poll at 250 ms;
untrusted input/write timeouts are 500 ms. Capture preparation runs only at
launch, outside presentation and monitoring callbacks. The wrapper waits at
most ten seconds for background startup plus ten seconds for the accepted
request; denial never retries by starting another app. Existing one-shot
activation, renderer-startup, process-scan and frame-transfer bounds still apply.
