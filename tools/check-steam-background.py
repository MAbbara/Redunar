"""Run native Steam startup probes on a private bus/display without host services.

Build steam_background_probe in .redunar-build/checks/tauri before running.
The private bus intentionally has no service activation directories or tray host.
"""

import os
import pathlib
import resource
import subprocess
import tempfile


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    workspace = pathlib.Path(__file__).resolve().parents[1]
    binary = workspace / ".redunar-build/checks/tauri/debug/examples/steam_background_probe"
    reports = workspace / ".redunar-build/reports"
    reports.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="redunar-steam-desktop-") as directory:
        root = pathlib.Path(directory)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        temporary = root / "tmp"
        temporary.mkdir(mode=0o700)
        config = root / "session-bus.conf"
        config.write_text(
            '<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen>'
            '<policy context="default"><allow send_destination="*"/>'
            '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
        )
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), TMPDIR=str(temporary), GIO_USE_VFS="local",
                   GTK_USE_PORTAL="0", GDK_BACKEND="x11", REDUNAR_PRIVATE_DESKTOP_FIXTURE="1")
        # Sequential displays avoid xvfb-run's automatic display allocation race.
        cases = [(case, [f"--{case}"]) for case in (
            "tray-off", "tray-fallback", "tray-off-auto-focus", "tray-off-key", "tray-off-click", "tray-off-host", "tray-on-host")]
        cases.extend([
            ("tray-off-reopen", ["--tray-off-host", "--reopen"]),
            ("tray-on-reopen", ["--tray-on-host", "--reopen"]),
            ("manual-owner-reopen", ["--tray-off-host", "--reopen", "--manual-owner"]),
            ("tray-off-park", ["--tray-off-host", "--park"]),
            ("tray-on-park", ["--tray-on-host", "--park"]),
            ("manual-owner-park", ["--tray-off-host", "--park", "--manual-owner"]),
            ("tray-off-unavailable-close", ["--tray-off", "--park"]),
            ("tray-off-canceled-close", ["--tray-off", "--cancel-close"]),
        ])
        for case, arguments in cases:
            result = subprocess.run(
                ["dbus-run-session", "--config-file", str(config), "--",
                 "xvfb-run", "-a", str(binary), *arguments],
                env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                timeout=20, check=False,
            )
            report = reports / f"steam-background-{case}.log"
            report.write_bytes(result.stdout)
            if result.returncode:
                raise RuntimeError(f"{case} failed ({result.returncode}); see {report}")
            print(f"PASS: {case}; report: {report}")


if __name__ == "__main__":
    main()
