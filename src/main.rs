#![allow(missing_docs)]

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "skillet", about = "Skill management CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a skillet workspace
    Init {
        /// Adopt existing SKILL.md files as .skill sources
        #[arg(long)]
        adopt: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { adopt } => {
            let cwd = std::env::current_dir()?;
            skillet::init::run(&cwd, adopt)?;
        }
    }
    Ok(())
}
