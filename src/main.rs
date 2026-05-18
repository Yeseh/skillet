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
    /// Show token budget for skills in the workspace
    Budget {
        /// Name of a single skill to show (shows all if omitted)
        name: Option<String>,
        /// Output format: human (default) or json
        #[arg(long, default_value = "human")]
        format: String,
    },
    /// Check skills for quality issues
    Lint {
        /// Name of a single skill to lint (lints all if omitted)
        name: Option<String>,
        /// Promote warnings to errors
        #[arg(long)]
        strict: bool,
        /// Show info-level diagnostics
        #[arg(long)]
        pedantic: bool,
        /// Output format: human (default) or json
        #[arg(long, default_value = "human")]
        format: String,
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
        Commands::Budget { name, format } => {
            let cwd = std::env::current_dir()?;
            let fmt = match format.as_str() {
                "json" => skillet::budget::OutputFormat::Json,
                _ => skillet::budget::OutputFormat::Human,
            };
            skillet::budget::run(&cwd, name.as_deref(), fmt)?;
        }
        Commands::Lint { name, strict, pedantic, format } => {
            let cwd = std::env::current_dir()?;
            let fmt = match format.as_str() {
                "json" => skillet::lint::OutputFormat::Json,
                _ => skillet::lint::OutputFormat::Human,
            };
            let opts = skillet::lint::LintOptions::new(strict, pedantic, fmt);
            let clean = skillet::lint::run(&cwd, name.as_deref(), &opts)?;
            if !clean {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
