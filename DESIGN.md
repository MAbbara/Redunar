# Redunar interface contract

Current Tauri design, reconciled October 14, 2026. This file owns approved
visual and interaction rules. Source is in `output/tauri-redunar/ui/`.

## Shell and visual language

Navigation is Overview, Library, Global settings, Instant Replay, History, and
Settings. Center the Redunar wordmark in the expanded sidebar and Comet R in its
collapsed form, with clear page titles in the workspace. Flat section navigation does not need a repeated
Workspace breadcrumb bar. The sidebar can collapse to icons with accessible
labels, hover titles, and an explicit expansion control. Keep graphics subordinate
to measurements and controls. Keep marketing language, decorative motion, and
session-identity concepts out of the app workspace.

Use a black canvas, neutral charcoal panels, grey separators, and Redunar red
`#e7474f`. Avoid purple-tinted neutral surfaces and orange replacements for red.
The approved control colors remain independent of surface refinements. Inspect
the complete stylesheet order in `ui/index.html`; `app.css` alone is not the
rendered design. `workspace.css`, `controls.css`, the Precision styles, and the
scoped feedback/history/replay files refine the base.

Use the bundled Noto Sans for interface text and Noto Sans Mono for measurements
and timecodes. Preserve the shipped font notices. Icons retain a square aspect
ratio and must not shrink into squeezed shapes.

## Shared controls and spacing

- **Soft fill buttons:** quiet grey secondary surfaces, muted red primary fill
  with a brighter border, rounded corners. Keep destructive actions explicit.
- **Soft ring toggles:** outlined track and ring handle, with a larger hit area.
  Show enabled, disabled, hover, and keyboard-focus states distinctly.
- **Precision sliders:** thin filled track and outlined handle. Values, units,
  bounds, keyboard movement, and disabled state must remain readable.
- **Precision selectors:** reuse the shared native-select enhancement and its
  keyboard/typeahead, dismissal, focus restoration, and viewport placement.
- Keep at least 12 px between buttons and 16 px above/below standalone settings
  action rows. Preserve spacing when rows wrap. Supporting block text clears
  preceding components by at least 16 px, and save/discard areas by 24 px.
- Inputs and adjacent file-picker buttons align vertically. Text and controls
  must not touch dividers, card edges, or the component above them.
- Save/Discard appears only when there are unsaved changes, at the bottom of
  the visible workspace. Reserve space so it cannot cover content. Do not render
  an always-visible All changes saved bar or a duplicate inside Library.
- Dialog opening focuses a neutral dialog/title target rather than highlighting
  Close as the default action. Preserve accessible keyboard focus, Escape,
  focus containment, and restoration to the invoking control.

## Overview

Give the frame graph more space than the active-session banner. The chosen
session layout has a status footer: keep the timer beside End session and
separate profile, saved captures, and enabled-feature information. Do not show
Previous settings captured. No separate overlay-visibility action belongs here;
Global and per-game settings own that control.

System metrics groups CPU/GPU load, separate temperature meters, RAM, and VRAM
where available, with readable separators. Red shades follow the graph; missing
readings remain unavailable. Use Recent captures, not Recent moments, and
System metrics, not Under the hood.

## Library

Keep the compact searchable game catalog beside selected-game details. Use
poster artwork in the catalog and landscape banner artwork in the header;
never stretch a poster into a banner. Missing artwork has a neutral fallback.
Retain the game, profile, and cached artwork after uninstall. Disable Launch
game with small Not installed text underneath when the installation is missing.

Game settings and Launch matching are separate tabs. Clearly show inherited
versus custom values and Reset to global settings. Changes retain their own
drafts through navigation and unrelated saves. Installed-game import reviews
local Steam/XDG entries; running helper processes are not import candidates.
Overview offers **Add your first game**, with a short add/scan instruction and
**Open Library**, only after the catalog successfully loads with no saved games.
Loading and failed catalog reads never appear as a first-use empty state. Once
any game is saved, the guide disappears. Steam-specific setup stays beside the
selected Steam game in Library. Every
selected Steam game shows a **Play from Steam** panel above both Library tabs,
with one paste instruction, the exact Launch Options value, **Copy**, and a short
automatic startup note. Routine unconfigured and running-Steam states use this
same compact setup; no permanent retry button or technical inspection warning.
Verified configured games retain the copyable value with a quiet “Launch option
configured” and a Play instruction. Other ambiguous reasons, unavailable setup
and failed checks retain explanations and Check again. Setup confirmation never
implies capture is ready, and no Steam restart is required. Import the
game once to select its saved profile. Steam setup explains Play in Steam and
automatic background startup. An automatically started app stays hidden in the
tray, with a temporary icon even when Close to tray is disabled. Opening Redunar
removes that temporary icon and keeps the app available; the saved preference
is unchanged. An unavailable tray uses a minimized window fallback without
requesting focus. Manual startup shows a tray icon only when Close to tray is enabled.
An automatically started app exits after game exit and successful session/history
cleanup when Close to tray is disabled and the user never used its window.
Clicking, typing, touching or scrolling in Redunar, or choosing Open from its tray,
keeps it available. Focus alone does not: a tiling desktop can focus the window
automatically at startup or game exit. An already running app also stays running.
Cleanup failures show the window for attention. Startup never
changes the saved Close to tray preference.
Closing Redunar's window during a live game returns it to the tray and keeps
Replay/session ownership active, including with Close to tray disabled. With
that setting disabled, the closed-window owner quits after the game ends and
cleanup succeeds. Reopening keeps it available; closing again restores automatic
exit. An unavailable tray keeps the window visible. When no game is owned,
closing follows the saved preference normally. Explicit Quit remains available
in the tray and ends Redunar's session without killing the game.
Executable, literal arguments (one per line), and working directory use the
native validation flow. Do not turn launch arguments into a shell command.

## Global settings and Settings

Global settings owns frame-metric collection, overlay visibility/appearance,
recording configuration, replay folder/menu preferences, and optional shortcuts.
Per-game controls override the supported subset with explicit inheritance.
Saving overlay visibility also updates a prepared running game according to
its inherited/custom setting; hiding must not remove the runtime.

Overlay preview reflects the real renderer's layout, metric order, scale, and
background-only opacity over a soft gradient. Compact, FPS only, Detailed, and
Custom previews show their actual geometry. Custom selections remain valid and
nonempty. Grid, Ribbon, and Telemetry change the structure independently of the
information preset and Custom metric selection. The bounded palettes are
Redunar, Glacier, Ember, Mint, Mono, Amethyst, Solar, and Rose. The preview must
use the same palette roles, rounded panel bounds, and layout dimensions as the
in-game renderer. Redunar branding is an optional global appearance setting;
its saved value updates a prepared running overlay through the same live path
as palette and layout changes. Renderer ABI details belong in
architecture/component documentation.

Automatic replay has no global/per-game Off selector. Recording mode and quality
configuration and the global microphone toggle are locked through launch,
recording and teardown; this must not block an
otherwise valid visibility-only save. Microphone help states that changes apply
to the next game launch, and failed writes restore the saved toggle value.
Shared inactive Replay guidance says “Launch a supported game to build a local
buffer.” on Overview and Instant Replay; it does not restrict the launcher.
Refresh inputs followed by Microphone input form one control group aligned at the
right of a wide settings row, below its label on narrow screens. Refresh inputs
uses a neutral secondary button. With Microphone off, explain how to enable
input selection; during a game, explain the session lock. Discovery errors and
missing-input feedback occupy a separate full-width line.
The Replay panel retains the recording-folder control and its resolved path.
The MKV/MP4 save container may change
during a session and applies to future saves. Menu/save shortcuts may all be
cleared. Global Replay settings offer short **60 FPS** and **Variable FPS**
choices. Older fixed 30/120 FPS profiles remain readable and distinct. Variable
mode follows accepted game presents within hardware and resource limits; it is
not a display-scanout measurement or a promise to capture every visible frame.

Keyboard shortcuts offers Retry activation when saved bindings exist but the
helper is inactive or unavailable. Retrying does not save or discard pending
shortcut/profile drafts. The status message updates with the native activation
result and retains useful input-access errors after a helper exit.

Settings contains the current application preferences, including Close to tray
and software updates. Automatic update checks default on; the user can disable
them or request a manual check. Automatic checks do not install packages.
Close to tray and Debug log share one Preferences card. Feature access and
software update controls remain together beside it, or below on narrow layouts.
When a verified update package is already cached, an automatic check keeps it
available. A manual check refreshes the signed release channel so a newer
version can replace that pending package; an unavailable channel leaves the
verified cached package available with an honest status.
When a startup check finds an update, direct the user to Settings to install it;
keep package-verification details in the Settings status. Installation always
requires a clear user action and remains native-owned. Update now requests
system authorization through polkit and installs the signed package through
the system package manager. Cancellation keeps the verified package ready to
retry. Settings reports completion only after the installed package version
is confirmed and says when a restart is needed.
Group a compact updater icon, state heading and action on one row;
narrow layouts move actions below the heading. Feature access and Check on
startup use evenly spaced rows below one separator. Keep native errors, retryable
installer reasons visible without clipping. Update now stays aligned like Check
for updates, with an accessible refresh icon immediately to its left. Omit the
version below Update available and the routine blocked-restart sentence; restart
still remains disabled during a running session. The
sidebar keeps the labelled Collapse sidebar control and one Settings navigation
row. Show the running version as plain text on the right of Settings, within
the same link. No separate version bar or sidebar install/restart actions remain.
A small, vertically centered icon to the left of the version indicates an
available update or required restart. Keep a 7px gap and normal text contrast
so the icon never overlaps or dims the version. In icon-only navigation, place the badge on the Settings gear.
The badge is passive; checking, installing and restarting stay in Software updates.
Installation/restart progress is indeterminate;
the version changes only when the newly installed executable starts. Restart asks to save
or discard any global, shortcut or per-game drafts; failed saves retain pending
edits and prevent restart. Feature access replaces the former Beta access toggle
with Stable/Beta choices backed by the same saved preference and restart
semantics. Stable is the default. This selector does not change update sources. The Debug
log description omits the routine Logging is off sentence, while active/error
status stays visible. Audio status dots in Overview
and Replay are green while capturing, red when unavailable, and grey when inactive.
Close to tray immediately controls tray icon visibility and closing behavior.
Steam automatic startup and live-game window close have the temporary-icon
exceptions described above.
The native StatusNotifier tray needs a desktop tray host. Closing hides only
while Redunar can verify its own usable registration; tray loss restores the
window. KDE and DMS provide compatible hosts; GNOME needs a compatible extension.
No AppIndicator library installation is required.
Keep this page limited to current user-configurable behavior and runtime
diagnostics.

Settings also owns the optional Debug log switch. It is off by default and
takes effect on the next Redunar start. When enabled, Redunar records its
operational messages, timestamped, into a private bounded file in the state
directory, rotating to at most two bounded files. The saved switch is distinct
from current logger health: Settings omits the routine off sentence and reports active, restart needed, or
unavailable, including a write failure after startup. The settings copy states
plainly that the file can include game names and session details, and offers
an Open log folder action once the file exists. The log never contains clip
media. Sharing it is always the owner's explicit choice. Beta access never
enables logging implicitly. See the [tester workflow](TESTING.md#nvidia-beta-first-hardware-test).

Settings offers Feature access with Stable/Beta choices. Stable maps to the
existing disabled Beta access preference and is the default. Anyone may opt in to early
features included in the installed build; it does not fetch code, change the
update channel, enable Debug log, or override a feature's native capability
checks. The preference is owned by the service and applies after an app restart
and a new game launch. In this build it admits experimental NVIDIA GPU metrics
and a single-GPU NVIDIA Vulkan Video Replay candidate. Hardware and codec
readiness still determine availability; the NVIDIA paths have not been verified
on NVIDIA hardware.

## Instant Replay

Use a player/editor beside an independently scrolling saved-clip rail. Selection,
loading, retry, trashing, restoration, and inventory refresh must preserve appropriate scroll,
search, focus, and save-duration state. Long filenames wrap or truncate without
forcing horizontal scrolling; show available game, duration, date, and size.

Filename editing exposes the name without its extension and preserves the original
file type automatically. Overview and Replay use a shared capture status treatment;
readiness, waiting and failures remain distinct, with complete failure explanations.
Both pages offer the same duration and Save replay controls, retaining the selected
duration when switching pages and displaying its saved shortcut. Keep the
Overview capture bar focused on saving; the sidebar opens the Replay editor.

The video area's aspect ratio follows the decoded clip. Keep the entire image
visible without cropping, rounded boundaries, and playback controls directly
underneath. Do not crop video just to remove letterboxing; black pixels recorded
inside the file are part of the footage. Keep technical video details at top
right, the actual filename in the bar above the player, and the game name
once at bottom left. Playback volume and mute controls are omitted so they
cannot be mistaken for changes to exported audio. No Storyboard preview caption
or expand control belongs in production.

The filmstrip has a time ruler, In/Out handles, playhead, selected duration, and
Export selection. The export dialog shows In, Out, and Duration in separated
columns and explains that the original remains. Saving/exporting reports success
only after native completion; loading failures offer a useful retry/error state.

The shortcut-opened Replay menu is rendered inside the game. No desktop or in-app
preview is exposed because it cannot represent the production Vulkan presentation.
See REPLAY for compositor limitations.
Successful in-game saves use the bottom-left Moment saved pill with saved length
and Local library, even when the metrics display is hidden.
An unavailable Replay state uses one stable, actionable reason from native
capture or encoder status; an inactive recorder never implies a populated buffer.
Waiting for the first recordable frames during an active game is pending, rather
than a hardware error. Explicit source rejection or recorder failure remains an
error and saving stays disabled until a healthy buffer is available.
The app's Save recent gameplay row keeps the duration, Save replay button and
optional shortcut together. Duration changes preserve control positions; reserve
the longest saved duration shortcut's width only while save bindings exist.

The in-game metrics overlay and Replay menu follow the approved modern
reference: soft rounded panels, quiet letter-spaced labels above bright
values, thin 1 px dividers, a dark-red selected duration cell with a red
underline, and a red-outlined save button. The eight bounded palettes keep
their accent, muted, text, divider, and panel roles in both the injected
renderer and this preview; the Replay menu keeps fixed Redunar control colors
independent of the metric palette. Renderer geometry and palette facts live in
the capture crates and the shader notes under
`crates/redunar-capture-vulkan/src/shaders/README.md`.

## History and honest data

Preserve the native report layout with the approved sidebar/search. Graph labels,
traces, slider, cursor, and selected readings must share coordinates and real
recorded timestamps. Use Redunar red for CPU and lighter red `#ff9297` for GPU
traces and legend swatches; keep measurement-card text neutral. This rule is
scoped to History, not an instruction to recolor unrelated hardware UI.

Use the [calculation contract](output/tauri-redunar/HISTORY-CALCULATIONS.md).
Never smooth away recorded spikes, invent missing samples, or report a value
that contradicts its plotted height. Empty, partial, unavailable, and
compatibility data need honest states. A fixture verifies presentation, not
native measurement.

The saved-clip rail has text sorting with an arrow, a count, and individual clip
favorites. Trash bin sits beside folder and refresh actions. There is no multiple
selection. Trash confirmation names the file once and says “Trashed clips can be
restored from the trash bin.” Undo lasts 10 seconds; expiration does not delete
media. The bin offers individual Restore. Export contains Filename, the read-only
configured replay destination, Cancel and Export; trim times remain in the editor.
History combines text, game and date filters and clears stale reports on no match.

Global settings tabs and its sidebar entry mark retained unsaved drafts. Navigation
between app pages remains available without losing them. Immediately saved Replay
preferences carry Auto-saved labels. Layout, preset and palette each have one control
set beside the overlay preview. Supporting text is generally 12px with brighter
neutral contrast. Remove redundant technical descriptions rather than adding help
panels. The icon/wordmark, expanded-sidebar default and existing round graph joins
remain unchanged by this refinement.
