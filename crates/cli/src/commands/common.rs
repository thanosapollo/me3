use std::path::PathBuf;

use clap::{ArgAction, Args};

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
