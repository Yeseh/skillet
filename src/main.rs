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
    /// Scaffold a new skill source in the current workspace
    New {
        /// Name of the skill to create
        name: String,
    },
    /// Compile .skill sources to SKILL.md output files
    Build {
        /// Name of a single skill to compile (compiles all if omitted)
        name: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { adopt } => {
            let cwd = std::env::current_dir()?;
            skillet::init::run(&cwd, adopt)?;
        }
        Commands::New { name } => {
            let cwd = std::env::current_dir()?;
            skillet::new::run(&cwd, &name)?;
        }
        Commands::Build { name } => {
            let cwd = std::env::current_dir()?;
            skillet::build::run(&cwd, name.as_deref())?;
        }
    }
    Ok(())
}
