use clap::*;
use launch::LaunchArgs;
use profile::ProfileCommands;

pub mod common;
pub mod info;
pub mod launch;
pub mod profile;

pub mod wrap;

#[cfg(target_os = "windows")]
pub mod windows;

#[derive(Subcommand, Debug)]
#[command(flatten_help = true)]
pub enum Commands {
    /// Launch the selected game with mods.
    #[clap(disable_version_flag = true)]
    Launch(LaunchArgs),

    /// Show information on the me3 installation and search paths.
    #[clap(disable_version_flag = true, disable_help_flag = true)]
    Info,

    #[clap(subcommand, disable_version_flag = true)]
    Profile(ProfileCommands),

    /// Wrap a Steam %command% to inject mods. Usage: me3 wrap [OPTIONS] -- %command%
    #[clap(disable_version_flag = true)]
    Wrap(wrap::WrapArgs),

    #[cfg(target_os = "windows")]
    #[clap(hide = true)]
    AddToPath,

    #[cfg(target_os = "windows")]
    #[clap(hide = true)]
    RemoveFromPath,

    /// Check for and launch a new version of the me3 installer.
    #[cfg(target_os = "windows")]
    Update,
}
