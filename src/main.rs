mod config;
mod nexus;
mod ops;
mod tui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use nexus::NxmLink;
use ops::{CommandRunner, ConsoleReporter, OperationContext};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "kiss-me")]
#[command(about = "A small Cyberpunk-style mod enabler with CLI and TUI modes")]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[arg(long, global = true, value_name = "PROFILE")]
    profile: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Launch the interactive terminal UI.
    Tui,
    /// Print the resolved config path.
    ConfigPath,
    /// Write a starter config file if one does not exist.
    InitConfig,
    /// Build a fresh modded game directory.
    Assemble,
    /// Unpack archives from Downloads into Library.
    ExtractDownloads,
    /// Enable a mod package.
    Enable { mod_name: String },
    /// Disable a mod package.
    Disable { mod_name: String },
    /// Check enabled mod directory structure.
    CheckLibrary,
    /// Pack local game changes as an overwrite mod.
    GenerateOverwrite,
    /// Record the current modded game state.
    SaveManifest,
    /// Show changes since the saved manifest.
    DiffManifest,
    /// Remove top-level .txt files from mod packages.
    RemoveReadmes,
    /// Download a Nexus Mods nxm:// link into Downloads.
    Download { nxm_url: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Some(Command::ConfigPath)) {
        println!("{}", Config::path(cli.config.as_deref())?.display());
        return Ok(());
    }

    if matches!(cli.command, Some(Command::InitConfig)) {
        let path = Config::write_starter(cli.config.as_deref())?;
        println!("Wrote starter config: {}", path.display());
        return Ok(());
    }

    let config = Config::load(cli.config.as_deref())?;
    let profile = config.active_profile(cli.profile.as_deref())?;
    let mut reporter = ConsoleReporter;
    let ctx = OperationContext::new(profile.clone(), CommandRunner::real());

    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => tui::run(config, profile)?,
        Command::ConfigPath | Command::InitConfig => unreachable!("handled before config load"),
        Command::Assemble => ops::assemble(&ctx, &mut reporter)?,
        Command::ExtractDownloads => ops::extract_downloads(&ctx, &mut reporter)?,
        Command::Enable { mod_name } => ops::enable_mod(&ctx, &mod_name, &mut reporter)?,
        Command::Disable { mod_name } => ops::disable_mod(&ctx, &mod_name, &mut reporter)?,
        Command::CheckLibrary => ops::check_library(&ctx, &mut reporter)?,
        Command::GenerateOverwrite => ops::generate_overwrite(&ctx, false, &mut reporter)?,
        Command::SaveManifest => ops::save_manifest(&ctx, &mut reporter)?,
        Command::DiffManifest => {
            for line in ops::manifest_changes(&ctx)? {
                println!("{line}");
            }
        }
        Command::RemoveReadmes => ops::remove_readmes(&ctx, &mut reporter)?,
        Command::Download { nxm_url } => {
            let link = NxmLink::parse(&nxm_url).context("invalid nxm:// link")?;
            let api_key = config
                .nexus_api_key()
                .or_else(|| std::env::var("NEXUS_API_KEY").ok())
                .or_else(|| std::env::var("NEXUSMODS_API_KEY").ok())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing Nexus API key; set nexus_api_key in config or NEXUS_API_KEY"))?;

            if let Some(expected) = profile.nexus_game_domain.as_deref() {
                if expected != link.game_domain {
                    bail!(
                        "nxm game domain '{}' does not match active profile domain '{}'",
                        link.game_domain,
                        expected
                    );
                }
            }

            nexus::download_nxm(&profile, &api_key, &link, &mut reporter)?;
        }
    }

    Ok(())
}
