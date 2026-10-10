use redunar_platform::{
    SteamAppId, ensure_steam_session, resolve_steam_wrapper_environment, steam_broker_socket_path,
};
use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let Some(invocation) = Invocation::parse(env::args_os().skip(1)) else {
        eprintln!("usage: redunar-steam-launch --app-id N -- command [arguments...]");
        return ExitCode::from(64);
    };

    let mut managed = BTreeMap::new();
    if let (Some(app_id), Some(socket_path)) = (invocation.app_id, broker_socket_path()) {
        let inherited = env::vars_os().collect::<BTreeMap<OsString, OsString>>();
        managed = resolve_steam_wrapper_environment(app_id, &socket_path, &inherited);
        if managed.is_empty()
            && let Some((runtime, app)) = background_app_paths()
        {
            match ensure_steam_session(app_id, &runtime, &app) {
                Ok(true) => {
                    managed = resolve_steam_wrapper_environment(app_id, &socket_path, &inherited);
                }
                Ok(false) => {
                    eprintln!(
                        "redunar-steam-launch: session unavailable or another game is active"
                    );
                }
                Err(_) => eprintln!(
                    "redunar-steam-launch: background session unavailable; continuing game launch"
                ),
            }
        }
    }
    if is_proton_invocation(&invocation) {
        remove_opengl_preload(&mut managed);
    }
    let managed_value = |name: &str| managed.get(&OsString::from(name)).cloned();
    let use_gamescope = managed_value("REDUNAR_GAMESCOPE")
        .or_else(|| env::var_os("REDUNAR_GAMESCOPE"))
        .and_then(|value| value.to_str().map(|value| value == "1"))
        .unwrap_or(false)
        && std::process::Command::new("gamescope")
            .arg("--help")
            .output()
            .is_ok();
    let use_gamemode = managed_value("REDUNAR_GAMEMODE")
        .or_else(|| env::var_os("REDUNAR_GAMEMODE"))
        .is_some_and(|value| value == "1")
        && std::process::Command::new("gamemoderun")
            .arg("--version")
            .output()
            .is_ok();
    let mut command = if use_gamescope {
        let mut command = Command::new("gamescope");
        command.arg("-f");
        command
            .arg("--")
            .arg(&invocation.command)
            .args(&invocation.arguments);
        command
    } else if use_gamemode {
        let mut command = Command::new("gamemoderun");
        command.arg(&invocation.command).args(&invocation.arguments);
        command
    } else {
        let mut command = Command::new(&invocation.command);
        command.args(&invocation.arguments);
        command
    };

    for (name, value) in managed {
        command.env(name, value);
    }
    let error = command.exec();
    eprintln!("redunar-steam-launch could not start the original game command: {error}");
    ExitCode::from(if error.kind() == std::io::ErrorKind::NotFound {
        127
    } else {
        126
    })
}

fn is_proton_invocation(invocation: &Invocation) -> bool {
    std::iter::once(&invocation.command)
        .chain(invocation.arguments.iter())
        .filter_map(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .any(|value| {
            value == "proton"
                || value.ends_with("/proton")
                || value.contains("/pressure-vessel")
                || value.contains("/steam-runtime")
        })
}

fn remove_opengl_preload(managed: &mut BTreeMap<OsString, OsString>) {
    let Some(preload) = managed.get(OsStr::new("LD_PRELOAD")).cloned() else {
        return;
    };
    let retained = std::env::split_paths(&preload)
        .filter(|path| path.file_name() != Some(OsStr::new("libredunar_capture_opengl.so")))
        .collect::<Vec<_>>();
    match std::env::join_paths(retained) {
        Ok(value) if !value.is_empty() => {
            managed.insert(OsString::from("LD_PRELOAD"), value);
        }
        _ => {
            managed.remove(OsStr::new("LD_PRELOAD"));
        }
    }
}

fn broker_socket_path() -> Option<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| steam_broker_socket_path(&path))
}

fn background_app_paths() -> Option<(PathBuf, PathBuf)> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())?;
    let executable = env::current_exe().ok()?;
    Some((runtime, executable.parent()?.join("redunar-tauri")))
}

struct Invocation {
    app_id: Option<SteamAppId>,
    command: OsString,
    arguments: Vec<OsString>,
}

impl Invocation {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> Option<Self> {
        let arguments = arguments.into_iter().collect::<Vec<_>>();
        let separator = arguments
            .iter()
            .position(|argument| argument == OsStr::new("--"))?;
        let command = arguments.get(separator.checked_add(1)?)?.clone();
        let command_arguments = arguments.get(separator + 2..)?.to_vec();

        // A malformed Redunar prefix must not reinterpret or shell-parse the
        // Steam-provided command. It simply disables activation and execs the
        // argv following `--` unchanged.
        let app_id = match arguments.get(..separator) {
            Some([option, value]) if option == OsStr::new("--app-id") => value
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
                .and_then(SteamAppId::new),
            _ => None,
        };

        Some(Self {
            app_id,
            command,
            arguments: command_arguments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    #[test]
    fn parser_preserves_non_utf8_steam_argv_without_shell_processing() {
        let command = OsString::from_vec(b"/games/non-utf8-\xff".to_vec());
        let literal = OsString::from("$(touch /tmp/redunar-wrapper-must-not-run)");
        let non_utf8 = OsString::from_vec(vec![b'-', b'-', 0xfe]);
        let invocation = Invocation::parse([
            OsString::from("--app-id"),
            OsString::from("1808500"),
            OsString::from("--"),
            command.clone(),
            literal.clone(),
            non_utf8.clone(),
        ])
        .expect("valid wrapper invocation");

        assert_eq!(invocation.app_id, SteamAppId::new(1_808_500));
        assert_eq!(invocation.command.as_bytes(), command.as_bytes());
        assert_eq!(invocation.arguments, [literal, non_utf8]);
    }

    #[test]
    fn malformed_prefix_fails_open_to_the_original_argv() {
        let invocation = Invocation::parse([
            OsString::from("--app-id"),
            OsString::from("not-a-number"),
            OsString::from("--"),
            OsString::from("/games/example"),
            OsString::from("--literal"),
        ])
        .expect("original command remains available");

        assert_eq!(invocation.app_id, None);
        assert_eq!(invocation.command, OsStr::new("/games/example"));
        assert_eq!(invocation.arguments, [OsString::from("--literal")]);
    }

    #[test]
    fn missing_original_command_is_rejected() {
        assert!(
            Invocation::parse([
                OsString::from("--app-id"),
                OsString::from("12"),
                OsString::from("--"),
            ])
            .is_none()
        );
    }

    #[test]
    fn proton_commands_disable_only_redunar_opengl_preload() {
        let invocation = Invocation {
            app_id: None,
            command: OsString::from("/steam/compatibilitytools.d/Proton/proton"),
            arguments: Vec::new(),
        };
        assert!(is_proton_invocation(&invocation));
        let mut managed = [(
            OsString::from("LD_PRELOAD"),
            OsString::from("/run/redunar/libredunar_capture_opengl.so:/usr/lib/game.so"),
        )]
        .into_iter()
        .collect();
        remove_opengl_preload(&mut managed);
        assert_eq!(
            managed.get(OsStr::new("LD_PRELOAD")),
            Some(&OsString::from("/usr/lib/game.so"))
        );
    }

    #[test]
    fn native_commands_keep_redunar_opengl_preload() {
        let invocation = Invocation {
            app_id: None,
            command: OsString::from("/games/native-game"),
            arguments: Vec::new(),
        };
        assert!(!is_proton_invocation(&invocation));
    }
}
