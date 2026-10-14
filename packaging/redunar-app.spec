Name:           redunar-app
Version:        0.1.23
Release:        1.local%{?dist}
Summary:        Steam gameplay capture and in-game metrics
License:        GPL-3.0-or-later
Source0:        redunar-app-package-root.tar.gz
BuildArch:      x86_64
Requires:       /usr/bin/pkexec
Requires:       /usr/bin/openssl
%if 0%{?suse_version}
Requires:       gtk3
Requires:       libwebkit2gtk-4_1-0
Requires:       libdrm2
Requires:       /usr/bin/parec
Requires:       /usr/bin/pactl
Requires:       libopus0
Requires:       /usr/bin/ffmpeg
Requires:       /usr/bin/ffprobe
Requires:       gstreamer-plugins-libav
Requires:       udev
%else
Requires:       gtk3
Requires:       webkit2gtk4.1
Requires:       libdrm
Requires:       /usr/bin/parec
Requires:       /usr/bin/pactl
Requires:       opus
# File/capability dependencies accept ffmpeg or ffmpeg-free, including RPM
# Fusion's standalone freeworld codec library or its full ffmpeg-libs provider.
# Do not pin a package name or swap an already compatible multimedia stack.
Requires:       /usr/bin/ffmpeg
Requires:       /usr/bin/ffprobe
Requires:       libavcodec-freeworld%{?_isa}
# WebKit consumes media through GStreamer; FFmpeg CLI tools alone do not
# install its decoder plugin. This plugin uses the compatible system codecs.
Requires:       %{_libdir}/gstreamer-1.0/libgstlibav.so
%endif

# This binary RPM packages already-built local Tauri artifacts.
# A future source RPM must build from a signed source archive with
# vendored/audited Rust dependencies instead.
%global debug_package %{nil}

%description
Redunar's native Tauri workspace for Steam gameplay capture and in-game metrics
on Linux with AMD hardware.

%prep
%setup -q -n redunar-app-package-root

%build

%install
mkdir -p %{buildroot}
cp -a usr %{buildroot}/

# Reload the rule and retrigger existing keyboard and mouse devices after
# installation. This removes the need to reconnect devices while keeping the
# runtime shortcut helper unprivileged.
%post
if [ -x /usr/bin/udevadm ]; then
    /usr/bin/udevadm control --reload-rules >/dev/null 2>&1 || :
    /usr/bin/udevadm trigger --subsystem-match=input --sysname-match=event* --property-match=ID_INPUT_KEYBOARD=1 --action=change >/dev/null 2>&1 || :
    /usr/bin/udevadm trigger --subsystem-match=input --sysname-match=event* --property-match=ID_INPUT_MOUSE=1 --action=change >/dev/null 2>&1 || :
fi
:

%files
%attr(0755,root,root) /usr/bin/redunar-tauri
%attr(0755,root,root) /usr/bin/redunar-steam-launch
%attr(0755,root,root) /usr/bin/libredunar_capture_vulkan.so
%attr(0755,root,root) /usr/bin/libredunar_capture_opengl.so
%attr(0755,root,root) /usr/libexec/redunar-hotkey-helper
%attr(0755,root,root) /usr/libexec/redunar-update-helper
%attr(0644,root,root) /usr/share/polkit-1/actions/com.redunar.install-update.policy
%attr(0644,root,root) /usr/share/applications/com.redunar.Redunar.desktop
%attr(0644,root,root) /usr/share/metainfo/com.redunar.Redunar.metainfo.xml
%attr(0644,root,root) /usr/lib/udev/rules.d/70-redunar-hotkeys.rules
%attr(0644,root,root) /usr/share/icons/hicolor/scalable/apps/com.redunar.Redunar.svg
%attr(0644,root,root) /usr/share/icons/hicolor/16x16/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/32x32/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/48x48/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/64x64/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/128x128/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/256x256/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/512x512/apps/com.redunar.Redunar.png
%attr(0644,root,root) /usr/share/icons/hicolor/1024x1024/apps/com.redunar.Redunar.png
%dir %attr(0755,root,root) /usr/share/doc/redunar
%attr(0644,root,root) /usr/share/doc/redunar/LICENSE-Lucide.txt
%attr(0644,root,root) /usr/share/doc/redunar/LICENSE-libc-MIT.txt
%attr(0644,root,root) /usr/share/doc/redunar/LICENSE-libc-APACHE.txt
%attr(0644,root,root) /usr/share/doc/redunar/LICENSE-Tauri-API-MIT.txt
%attr(0644,root,root) /usr/share/doc/redunar/LICENSE-Tauri-API-APACHE-2.0.txt
%attr(0644,root,root) /usr/share/doc/redunar/LICENSE-Noto-Sans-Mono.txt
%attr(0644,root,root) /usr/share/doc/redunar/THIRD-PARTY-NOTICES.md
%attr(0644,root,root) /usr/share/doc/redunar/DEPENDENCY-LICENSES.md
%dir %attr(0755,root,root) /usr/share/licenses/redunar
%license /usr/share/licenses/redunar/LICENSE
%license /usr/share/licenses/redunar/COPYRIGHT

%changelog
* Wed Oct 14 2026 Redunar <local@redunar.invalid> - 0.1.23-1.local
- Align microphone input controls and clarify disabled selection.
- Use generic Replay launch guidance on Overview and Instant Replay.

* Wed Oct 14 2026 Redunar <local@redunar.invalid> - 0.1.22-1.local
- Update the workspace, clip management, and capture controls.
- Reopen the existing Redunar window on repeated launches.
- Fix clip previews and Steam library import feedback.

* Sun Oct 11 2026 Redunar <local@redunar.invalid> - 0.1.21-1.local
- Fix Steam Play launches on X11 while preserving native capture.
- Add optional microphone recording mixed with game sound in Replay clips.
- Reduce replay gaps caused by short encoder bursts overflowing the recording queue.

* Fri Oct 09 2026 Redunar <local@redunar.invalid> - 0.1.20-1.local
- Keep Replay save controls aligned across durations and distinguish pending frames from errors
- Complete bounded Vulkan extension discovery for capture and the NVIDIA beta encoder

* Fri Oct 09 2026 Redunar <local@redunar.invalid> - 0.1.19-1.local
- Use a temporary tray icon for automatic Steam startup and live-game window close.
- Quit background owners after game cleanup when Close to tray is disabled.
- Ignore compositor focus when retaining an automatic owner.

* Fri Oct 09 2026 Redunar <local@redunar.invalid> - 0.1.18-1.local
- Fix the in-game Replay menu closing immediately after opening.
- Refresh mouse devices and discard stale input before menu interaction.

* Fri Oct 09 2026 Redunar <local@redunar.invalid> - 0.1.17-1.local
- Qualify native tray and compatibility fixes with bounded bus-disconnect fixtures.

* Fri Oct 09 2026 Redunar <local@redunar.invalid> - 0.1.16-1.local
- Prepare the native tray and compatibility fixes with general X11 release wording.

* Fri Oct 09 2026 Redunar <local@redunar.invalid> - 0.1.15-1.local
- Use a native StatusNotifier tray without AppIndicator.
- Accept ultrawide Replay sources within the existing capture budget.
- Read Intel package temperature and preserve private GLX provider forwarding.

* Tue Oct 06 2026 Redunar <local@redunar.invalid> - 0.1.14-1.local
- Prepare capture when configured games start from Steam, including background app startup.
- Simplify Steam setup and release game ownership after natural exit.
- Match Replay to the game's GPU and allow identifiable NVIDIA hybrid beta systems.

* Sat Oct 03 2026 Redunar <local@redunar.invalid> - 0.1.13-1.local
- Improve Replay saving, playback timing, shortcut recovery, and producer handoff.
- Harden NVIDIA beta capture and metrics and improve private diagnostic logging.

* Sat Sep 26 2026 Redunar <local@redunar.invalid> - 0.1.10-1.local
- Fix Replay menu visibility and audio timing in Instant Replay clips.

* Sat Sep 26 2026 Redunar <local@redunar.invalid> - 0.1.9-1.local
- Open the diagnostic log folder from Settings when requested.

* Sat Sep 26 2026 Redunar <local@redunar.invalid> - 0.1.8-1.local
- Add Variable FPS Replay recording alongside fixed 60 FPS.
- Improve NVIDIA Replay beta diagnostics while limiting shared log details.
- Retry frame release acknowledgements and keep NVIDIA encoder selection vendor-specific.

* Fri Sep 25 2026 Redunar <local@redunar.invalid> - 0.1.7-1.local
- Install signed in-app updates through a fixed polkit helper with retryable cancellation and verified package status.

* Fri Sep 25 2026 Redunar <local@redunar.invalid> - 0.1.6-1.local
- Refresh Settings by grouping Close to tray, Beta access, and Debug log in one Preferences card, with updated switches and shorter descriptions.

* Fri Sep 25 2026 Redunar <local@redunar.invalid> - 0.1.5-1.local
- Refine the in-game metrics overlay and Replay menu
- Add opt-in beta NVIDIA GPU metrics and a single-GPU Replay candidate

* Thu Sep 24 2026 Redunar <local@redunar.invalid> - 0.1.4-1.local
- Add GLX/SDL OpenGL metrics, overlay, and supported Replay capture
- Prefer native PipeWire output capture to preserve game music
- Add opt-in rotating diagnostic logs for tester reports

* Sun Sep 20 2026 Redunar <local@redunar.invalid> - 0.1.3-1.local
- Redesign the in-game Replay menu around a compact two-row duration picker,
  live buffer status, a direct save action, and in-overlay MKV/MP4 selection
- Apply changed Replay shortcuts and output formats to the active session
- Add a compact titlebar independent of the Linux desktop compositor

* Sun Sep 20 2026 Redunar <local@redunar.invalid> - 0.1.2-1.local
- Record the active system output through PulseAudio or PipeWire and recover
  automatically from route changes, stalled capture, and backend failures
- Add live and persisted control over in-game overlay branding
- Remove the obsolete desktop Replay preview and controls card
- Allow bounded re-encoding overhead when exporting trimmed clips

* Sun Sep 20 2026 Redunar <local@redunar.invalid> - 0.1.1-1.local
- Record the active system output through PulseAudio or PipeWire, reconnect
  stalled streams, and expose audio readiness in the Instant Replay workspace
- Preserve clip game attribution, cancel stale playback preparation, and guard
  game discovery and launch edits against stale UI state
- Correct overlay preview palette borders and remove obsolete generated assets

* Thu Sep 17 2026 Redunar <local@redunar.invalid> - 0.1.0-3.70.local
- Initial release with signed in-app update metadata, Instant Replay saves sized
  from the requested clip duration, and completed sessions cleared from Overview

* Tue Sep 15 2026 Redunar <local@redunar.invalid> - 0.1.0-3.68.local
- Adopt the permanent com.redunar.Redunar desktop, AppStream, icon, and window identity
- Preserve shared Redunar state while replacing the private Tauri launcher identity

* Mon Sep 14 2026 Redunar <local@redunar.invalid> - 0.1.0-3.67.local
- Ship the GPL project license, copyright notice, and locked dependency inventory
- Enforce the reviewed x86_64 Linux dependency-license policy for release checks

* Sun Sep 13 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.66.local
- Require media tools and codec capability without pinning an FFmpeg package provider
- Check installed encoder capabilities and preserve compatible RPM Fusion stacks

* Sun Sep 13 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.47.local
- Use local Steam hero/header artwork for Library banners, with separate poster caching
- Keep banner imagery out of layout flow and retain neutral backgrounds when unavailable

* Sun Sep 13 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.46.local
- Ship the current workspace, shared controls, replay editor and session footer
- Show cached RAM and VRAM readings and optional local Steam posters
- Preserve native profile and replay contracts; fix toggle boolean and launch argument round-trips

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.42.local
- Remove the duplicate in-app window controls and use native window chrome only

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.39.local
- Ship the freshly embedded frontend without custom main-window controls

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.38.local
- Rebuild the RPM from the current frontend with default native window chrome

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.37.local
- Restore default decorated window controls and native resize behavior

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.36.local
- Reduce WebKit repaint work while the native window is being resized

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.35.local
- Enlarge and offset native corner resize hit areas for reliable cursor feedback

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.34.local
- Add explicit corner resize handles for the frameless native window

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.33.local
- Remove duplicate header drag invocation and show native window controls immediately

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.32.local
- Explicitly select the default Tauri capability for custom window controls

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.31.local
- Allow custom window controls and header dragging through the Tauri window ACL

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.30.local
- Restore custom header dragging and tighten window control alignment

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.29.local
- Apply undecorated window state explicitly during native startup

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.28.local
- Remove the native title bar and use the in-app window controls

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.27.local
- Restrict the active capture and monitoring path to AMD GPUs

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.26.local
- Add in-app minimize, maximize/restore, and close window controls

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.25.local
- Preserve the click user gesture when starting Replay playback after a seek

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.24.local
- Make the Replay play/pause control respond reliably to pointer clicks and keyboard activation
- Keep the player toggle visible with an enlarged, explicit hit target

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.23.local
- Make Instant Replay open responsively and enlarge the player controls for reliable playback
- Anchor History traces at the session start when the first observation arrives later

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.22.local
- Reserve enough space for nine History rows before the session list begins scrolling

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.21.local
- Constrain the History session rail to a bounded nine-row viewport with internal scrolling

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.20.local
- Bound the History rail to its workspace height so excess entries scroll inside the panel

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.19.local
- Show newest History sessions first and confirm Steam sessions from accepted capture evidence
- Keep the History rail top anchored and refresh it whenever the backend journal revision changes

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.18.local
- Keep Overview frame pacing as a calm rolling timeline with Older and Now endpoints

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.17.local
- Show the product version in the sidebar instead of the internal RPM release

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.16.local
- Stretch the History session rail to the detail panel and inset its scrollbar
- Calm dense History traces with median time buckets while preserving exact scrubber samples

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.15.local
- Inject the current package build into the embedded sidebar version label

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.14.local
- Bound timeline area fills to the recorded sample span

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.13.local
- Avoid WebKitGTK DMA-BUF shutdown crashes and use the normal Tauri close lifecycle

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.12.local
- Keep the FPS History axis aligned to a stable 120 FPS reference scale

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.11.local
- Use production launcher tooltip copy and share the refined timeline chart treatment on Overview

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.10.local
- Refine History chart traces, remove the glow treatment, and cap the session list at nine visible rows

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.9.local
- Refine History timeline styling and use ended language for completed sessions

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.8.local
- Preserve numeric types when saving Global scale, opacity, and frame-rate settings

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.7.local
- Add a bounded full-session telemetry timeline with interactive FPS, temperature, and load inspection

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.6.local
- Allow Steam shader preparation up to five minutes before capture startup is considered failed

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.5.local
- Reduce startup and idle polling work while keeping live Replay views responsive

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.4.local
- Remove the saved-clip storage setting and quota; clips remain local until deleted or the filesystem safety reserve blocks a save

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.3.local
- Enable Save and Discard actions only when their settings have changed
- Align the Replay editor to its content and limit the desktop clip rail to four scrollable cards

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.2.local
- Keep the replay menu status synchronized with the live replay runtime
- Remove the redundant relaunch instruction from the unavailable state

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.1.local
- Resolve the capture layer from development dependencies and clarify replay startup status

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-3.0.local
- Remove the replay menu's translucent shadow surface around the opaque panel

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-2.9.local
- Make the in-game Replay menu surface opaque for legibility over game scenes

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-2.8.local
- Resize the Global overlay preview when presets or displayed metrics change

* Sat Sep 12 2026 Redunar local development <local@redunar.invalid> - 0.1.0-2.7.local
- Apply saved Global overlay layout changes to the current capture session

* Fri Sep 11 2026 Redunar local development <local@redunar.invalid> - 0.1.0-2.6.local
- Align the global preview top row, row heights, and divider positions with the native overlay renderer

* Fri Sep 11 2026 Redunar local development <local@redunar.invalid> - 0.1.0-2.5.local
- Clarify preset-controlled metric selections and Custom layout validation in the native settings flow

* Fri Sep 11 2026 Redunar local development <local@redunar.invalid> - 0.1.0-2.4.local
- Synchronize Custom metric selections and preset-driven overlay previews in the native Tauri settings flow
- Rename the distributable package to redunar-app for the production-facing build
- Refresh existing keyboard devices during installation so uaccess applies without a reconnect
- Install the native application and private sidecars as one package
- Add a package layout for the active Tauri application and its private capture sidecars
- Grant the active desktop session access to keyboard event nodes for local Replay shortcuts
