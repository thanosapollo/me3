use std::path::{Path, PathBuf};

use clap::{ArgAction, Args};
use color_eyre::eyre::eyre;
use me3_launcher_attach_protocol::AttachConfig;
use me3_mod_protocol::{native::Native, package::Package};
use normpath::PathExt;

use crate::{
    commands::{launch::GameOptions, profile::ProfileOptions},
    db::profile::Profile,
    Game,
};

#[derive(Args, Debug)]
pub struct ModArgs {
    /// Suspend the game until a debugger is attached.
    #[clap(long("suspend"), action = ArgAction::SetTrue)]
    pub suspend: bool,

    /// Name of a profile in the me3 profile dir, or path to a ModProfile (TOML or JSON).
    #[arg(
        short('p'),
        long("profile"),
        help_heading = "Mod configuration",
        value_hint = clap::ValueHint::FilePath,
    )]
    pub profile: Option<String>,

    /// Path to package directory (asset override mod) [repeatable option]
    #[arg(
        long("package"),
        action = ArgAction::Append,
        help_heading = "Mod configuration",
        value_hint = clap::ValueHint::DirPath,
    )]
    pub packages: Vec<PathBuf>,

    /// Path to DLL file (native DLL mod) [repeatable option]
    #[arg(
        short('n'),
        long("native"),
        action = ArgAction::Append,
        help_heading = "Mod configuration",
        value_hint = clap::ValueHint::FilePath,
    )]
    pub natives: Vec<PathBuf>,

    /// Name of an alternative savefile to use (in the default savefile directory).
    #[arg(long("savefile"), help_heading = "Mod configuration")]
    pub savefile: Option<String>,
}

pub(crate) fn remap_slr_path(path: impl AsRef<Path>) -> PathBuf {
    // <https://gitlab.steamos.cloud/steamrt/steam-runtime-tools/-/blob/4d85075e6240c839a3464fd97f22aa2253a9cea1/docs/shared-paths.md#never-shared>
    const NON_SHARED_PATHS: [&'static str; 4] = ["/usr", "/etc", "/bin", "/lib"];

    let path = path.as_ref();

    if NON_SHARED_PATHS
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        Path::new("/run/host").join(path.strip_prefix("/").unwrap())
    } else if path.starts_with("/app") {
        // Inside a Flatpak, pressure-vessel mounts the parent container's /app
        // at /run/parent/app rather than passing it through directly.
        Path::new("/run/parent").join(path.strip_prefix("/").unwrap())
    } else {
        path.to_path_buf()
    }
}

pub(crate) fn generate_attach_config(
    game: Game,
    opts: &GameOptions,
    profile: &Profile,
    profile_options: &ProfileOptions,
    cache_path: Option<Box<Path>>,
    extra_packages: &[PathBuf],
    extra_natives: &[PathBuf],
    savefile: Option<&str>,
    suspend: bool,
) -> color_eyre::Result<AttachConfig> {
    for path in extra_natives.iter().chain(extra_packages) {
        if !path.exists() {
            return Err(eyre!("{path:?} does not exist"));
        }
    }

    let mut packages = extra_packages
        .iter()
        .filter_map(|path| path.normalize().ok())
        .map(|normalized| Package::new(normalized.into_path_buf()))
        .collect::<Vec<_>>();

    let mut natives = extra_natives
        .iter()
        .filter_map(|path| path.normalize().ok())
        .map(|normalized| Native::new(normalized.into_path_buf()))
        .collect::<Vec<_>>();

    let (ordered_natives, early_natives, ordered_packages) = profile.compile()?;

    packages.extend(ordered_packages);
    natives.extend(ordered_natives);

    let savefile = savefile
        .map(|s| s.to_owned())
        .or_else(|| profile.savefile());

    if let Some(savefile) = &savefile {
        // https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file#naming-conventions
        let is_windows_path_reserved_char = |c: char| {
            matches!(
                c,
                '\x00'..'\x1f' | '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        };

        if savefile.chars().any(is_windows_path_reserved_char) {
            return Err(eyre!(
                "savefile name ({savefile:?}) contains reserved file name characters"
            ));
        }
    }

    Ok(AttachConfig {
        game: game.into(),
        packages,
        natives,
        early_natives,
        savefile,
        cache_path: cache_path.map(|path| path.into_path_buf()),
        suspend,
        boot_boost: opts.boot_boost.unwrap_or(true),
        skip_logos: opts.skip_logos.unwrap_or(true),
        start_online: profile_options.start_online.unwrap_or(false),
        disable_arxan: profile_options.disable_arxan.unwrap_or(false),
        mem_patch: !profile_options.no_mem_patch.unwrap_or(false),
        skip_steam_init: opts.skip_steam_init.unwrap_or(false),
    })
}
