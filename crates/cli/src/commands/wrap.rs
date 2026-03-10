use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

use clap::{ArgAction, Args};
use color_eyre::eyre::OptionExt;
#[cfg(unix)]
use color_eyre::eyre::bail;
use me3_env::{CommandExt, GameVars, LauncherVars, TelemetryVars};
use normpath::PathExt;
use tempfile::NamedTempFile;
use tracing::info;

use crate::{
    commands::{
        launch::{generate_attach_config, remap_slr_path, GameOptions, Selector},
        profile::ProfileOptions,
    },
    config::Config,
    db::{profile::Profile, DbContext},
    Game,
};

#[derive(Args, Debug)]
pub struct WrapArgs {
    #[clap(flatten)]
    target_selector: Option<Selector>,

    #[clap(flatten)]
    game_options: GameOptions,

    #[clap(flatten)]
    profile_options: ProfileOptions,

    /// Suspend the game until a debugger is attached.
    #[clap(long("suspend"), action = ArgAction::SetTrue)]
    suspend: bool,

    /// Name of a profile in the me3 profile dir, or path to a ModProfile (TOML or JSON).
    #[arg(
        short('p'),
        long("profile"),
        help_heading = "Mod configuration",
        value_hint = clap::ValueHint::FilePath,
    )]
    profile: Option<String>,

    /// Path to package directory (asset override mod) [repeatable option]
    #[arg(
        long("package"),
        action = ArgAction::Append,
        help_heading = "Mod configuration",
        value_hint = clap::ValueHint::DirPath,
    )]
    packages: Vec<PathBuf>,

    /// Path to DLL file (native DLL mod) [repeatable option]
    #[arg(
        short('n'),
        long("native"),
        action = ArgAction::Append,
        help_heading = "Mod configuration",
        value_hint = clap::ValueHint::FilePath,
    )]
    natives: Vec<PathBuf>,

    /// Name of an alternative savefile to use (in the default savefile directory).
    #[arg(long("savefile"), help_heading = "Mod configuration")]
    savefile: Option<String>,

    /// The Steam %command% to wrap. Usage: me3 wrap [OPTIONS] -- %command%
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

/// Find the game executable in the passthrough command args.
///
/// Steam/Proton commands may split paths containing spaces across multiple argv
/// entries. This walks backward from the last `.exe` argument, joining with
/// spaces, until a path that exists on disk is found.
///
/// Returns `(range, path)` where `range` is the index range in `args` that
/// comprises the exe path, and `path` is the resolved filesystem path.
fn find_game_exe(args: &[OsString]) -> Option<(std::ops::Range<usize>, PathBuf)> {
    // Find the last arg ending in .exe (case-insensitive)
    let exe_idx = args.iter().rposition(|arg| {
        arg.to_str()
            .is_some_and(|s| s.to_ascii_lowercase().ends_with(".exe"))
    })?;

    // Try progressively longer spans ending at exe_idx, joining with spaces.
    // This handles paths like "ELDEN RING NIGHTREIGN/Game/nightreign.exe" that
    // get split across multiple argv entries.
    for start in (0..=exe_idx).rev() {
        let candidate: String = args[start..=exe_idx]
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");

        let path = PathBuf::from(&candidate);
        if path.exists() {
            return Some((start..exe_idx + 1, path));
        }
    }

    // If no existing path found, return just the single .exe arg as-is.
    // This allows the command to proceed even if path validation fails
    // (e.g., the exe is inside a container mount that doesn't exist on the host).
    let path = PathBuf::from(&args[exe_idx]);
    Some((exe_idx..exe_idx + 1, path))
}

#[tracing::instrument(err, skip_all)]
pub fn wrap(db: DbContext, config: Config, args: WrapArgs) -> color_eyre::Result<()> {
    // Resolve game first: -g flag > SteamAppId env var (set by Steam before expanding %command%)
    let game = if let Some(selector) = &args.target_selector {
        selector
            .game
            .or_else(|| selector.steam_id.and_then(Game::from_app_id))
    } else {
        None
    }
    .or_else(|| {
        std::env::var("SteamAppId")
            .ok()
            .and_then(|id| id.parse::<u32>().ok())
            .and_then(Game::from_app_id)
    })
    .ok_or_eyre("unable to determine game: use -g or ensure SteamAppId is set")?;

    // Resolve profile: -p flag > config default_profile for this game > transient
    let default_profile = config
        .options
        .game
        .get(&game.into())
        .and_then(|opts| opts.default_profile.as_deref());

    let profile = if let Some(name) = args.profile.as_deref().or(default_profile) {
        db.profiles.load(name)?
    } else {
        Profile::transient()
    };

    info!(?game, profile = profile.name(), "wrap: resolved game");

    let game_options = config
        .options
        .game
        .get(&game.into())
        .cloned()
        .unwrap_or_default()
        .merge(args.game_options.clone());

    let profile_options = profile.options().merge(args.profile_options.clone());

    let attach_config = generate_attach_config(
        game,
        &game_options,
        &profile,
        &profile_options,
        config.cache_dir(),
        &args.packages,
        &args.natives,
        args.savefile.as_deref(),
        args.suspend,
    )?;

    let attach_config_dir = config.cache_dir().unwrap_or(Box::from(Path::new(".")));
    std::fs::create_dir_all(&attach_config_dir)?;
    let attach_config_file = NamedTempFile::new_in(&attach_config_dir)?;
    std::fs::write(&attach_config_file, toml::to_string_pretty(&attach_config)?)?;
    // On Unix, exec() replaces the process so the temp file is never cleaned up.
    // On Windows, the temp file is cleaned up when attach_config_path drops after wait().
    let attach_config_path = attach_config_file.into_temp_path();

    info!(?attach_config_path, ?attach_config, "wrote attach config");

    let bins_dir = config
        .windows_binaries_dir()
        .ok_or_eyre("can't find me3 Windows binaries directory")?;

    let launcher_path = if cfg!(target_os = "linux") {
        remap_slr_path(bins_dir.join("me3-launcher.exe"))
    } else {
        bins_dir.join("me3-launcher.exe")
    };
    let dll_path = if cfg!(target_os = "linux") {
        remap_slr_path(bins_dir.join("me3_mod_host.dll"))
    } else {
        bins_dir.join("me3_mod_host.dll")
    };

    let (exe_range, game_exe_path) = find_game_exe(&args.command)
        .ok_or_eyre("no .exe found in the passthrough command")?;

    info!(?game_exe_path, ?exe_range, "found game exe in command");

    let mut modified_args = args.command.clone();
    let launcher_os: OsString = launcher_path.as_os_str().to_owned();
    modified_args.splice(exe_range, [launcher_os]);

    // Resolve the actual game exe (e.g. nightreign.exe), not the EAC launcher
    // (start_protected_game.exe) that Steam's %command% points to.
    let game_exe = game_exe_path
        .ancestors()
        .find(|p| p.join(game.launcher()).exists())
        .map(|p| p.join(game.launcher()))
        .unwrap_or(game_exe_path.clone());

    let launcher_vars = LauncherVars {
        exe: game_exe,
        host_dll: dll_path,
        host_config_path: attach_config_path.to_path_buf(),
    };

    let log_file_path = db.logs.create_log_file(profile.name())?;
    let log_file_path = log_file_path
        .normalize()
        .map(|p| p.into_path_buf())
        .unwrap_or_else(|_| log_file_path.to_path_buf());

    let telemetry_vars = TelemetryVars {
        enabled: config.options.crash_reporting.unwrap_or_default(),
        log_file_path,
        // No monitor pipe needed: on Unix we exec(), on Windows we just wait.
        monitor_pipe_path: PathBuf::from(if cfg!(windows) { "NUL" } else { "/dev/null" }),
        trace_id: me3_telemetry::trace_id(),
    };

    let game_vars: GameVars = game.into_vars();

    let program = &modified_args[0];
    let rest = &modified_args[1..];

    let mut cmd = Command::new(program);
    cmd.args(rest);
    cmd.with_env_vars(game_vars);
    cmd.with_env_vars(launcher_vars);
    cmd.with_env_vars(telemetry_vars);

    info!(?cmd, "wrap: exec command");

    #[cfg(unix)]
    {
        // exec() replaces this process entirely. On success, this never returns.
        let err = cmd.exec();
        bail!("exec failed: {err}")
    }

    #[cfg(windows)]
    {
        let status = cmd.spawn()?.wait()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::Parser;

    use crate::{commands::Commands, Cli};

    #[test]
    fn wrap_parses_command_after_double_dash() {
        let cli = Cli::parse_from([
            "me3",
            "wrap",
            "-g",
            "nightreign",
            "-p",
            "my-profile",
            "--",
            "/path/to/reaper",
            "SteamLaunch",
            "AppId=2765620",
            "--",
            "/path/to/SLR/_v2-entry-point",
            "--verb=waitforexitandrun",
            "--",
            "/path/to/proton",
            "waitforexitandrun",
            "/path/to/game/nightreign.exe",
        ]);

        let Commands::Wrap(wrap_args) = cli.command else {
            panic!("expected Wrap command");
        };

        assert_eq!(wrap_args.profile.as_deref(), Some("my-profile"));
        assert_eq!(wrap_args.command.len(), 10);
        assert_eq!(
            wrap_args.command.last().unwrap(),
            &OsString::from("/path/to/game/nightreign.exe")
        );
    }

    #[test]
    fn find_game_exe_simple_path() {
        let args: Vec<OsString> = vec![
            "reaper".into(),
            "--".into(),
            "/path/to/proton".into(),
            "waitforexitandrun".into(),
            "/tmp/test.exe".into(),
        ];

        let result = super::find_game_exe(&args);
        assert!(result.is_some());
        let (range, _path) = result.unwrap();
        assert_eq!(range, 4..5);
    }
}
