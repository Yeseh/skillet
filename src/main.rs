#![allow(missing_docs)]

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

/// Output format accepted by subcommands that support `--format`.
#[derive(Clone, Default, ValueEnum)]
enum FormatArg {
    #[default]
    Human,
    Json,
}

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
        /// Output format
        #[arg(long, default_value = "human")]
        format: FormatArg,
    },
    /// Scaffold a new skill source in the current workspace
    New {
        /// Name of the skill to create
        name: String,
        /// Output format
        #[arg(long, default_value = "human")]
        format: FormatArg,
    },
    /// Compile .skill sources to SKILL.md output files
    Build {
        /// Name of a single skill to compile (compiles all if omitted)
        name: Option<String>,
        /// Disable URL verification regardless of config
        #[arg(long)]
        offline: bool,
        /// Promote URL-check warnings to errors
        #[arg(long)]
        strict: bool,
        /// Output format
        #[arg(long, default_value = "human")]
        format: FormatArg,
    },
    /// Show token budget for skills in the workspace
    Budget {
        /// Name of a single skill to show (shows all if omitted)
        name: Option<String>,
        /// Output format
        #[arg(long, default_value = "human")]
        format: FormatArg,
    },
    /// Verify compiled SKILL.md files are up-to-date with their sources
    Check {
        /// Output format
        #[arg(long, default_value = "human")]
        format: FormatArg,
    },
    /// Print a bundled skill's content to stdout
    Skill {
        /// Name of the bundled skill to print
        name: String,
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
        /// Output format
        #[arg(long, default_value = "human")]
        format: FormatArg,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { adopt, format } => {
            let cwd = std::env::current_dir()?;
            let json = matches!(format, FormatArg::Json);
            skillet::init::run(&cwd, adopt, json)?;
        }
        Commands::New { name, format } => {
            let cwd = std::env::current_dir()?;
            let json = matches!(format, FormatArg::Json);
            skillet::new::run(&cwd, &name, json)?;
        }
        Commands::Build {
            name,
            offline,
            strict,
            format,
        } => {
            let cwd = std::env::current_dir()?;
            let fmt = match format {
                FormatArg::Json => skillet::build::OutputFormat::Json,
                FormatArg::Human => skillet::build::OutputFormat::Human,
            };
            let opts = skillet::build::BuildOptions::new_with_format(offline, strict, fmt);
            skillet::build::run(&cwd, name.as_deref(), &opts)?;
        }
        Commands::Check { format } => {
            let cwd = std::env::current_dir()?;
            let fmt = match format {
                FormatArg::Json => skillet::check::OutputFormat::Json,
                FormatArg::Human => skillet::check::OutputFormat::Human,
            };
            let fresh = skillet::check::run(&cwd, fmt)?;
            if !fresh {
                std::process::exit(1);
            }
        }
        Commands::Budget { name, format } => {
            let cwd = std::env::current_dir()?;
            let fmt = match format {
                FormatArg::Json => skillet::budget::OutputFormat::Json,
                FormatArg::Human => skillet::budget::OutputFormat::Human,
            };
            skillet::budget::run(&cwd, name.as_deref(), fmt)?;
        }
        Commands::Skill { name } => {
            skillet::skill::run(&name)?;
        }
        Commands::Lint {
            name,
            strict,
            pedantic,
            format,
        } => {
            let cwd = std::env::current_dir()?;
            let fmt = match format {
                FormatArg::Json => skillet::lint::OutputFormat::Json,
                FormatArg::Human => skillet::lint::OutputFormat::Human,
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
