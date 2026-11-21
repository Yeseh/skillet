mod budget;
mod build;
mod check;
mod init;
mod lint;
mod new;
mod publish;
mod skill;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};

/// Output format accepted by subcommands that support `--format`.
#[derive(Clone, ValueEnum)]
enum FormatArg {
    Json,
}

/// Distribution target accepted by `skillet publish --target`.
#[derive(Clone, Copy, ValueEnum)]
enum PublishTargetArg {
    Direct,
    Apm,
}

#[derive(Parser)]
#[command(name = "skillet", about = "Skill management CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum SkillCommands {
    /// List all available bundled skills
    List,
    /// Print a bundled skill's content to stdout
    Print {
        /// Name of the bundled skill to print
        name: String,
    },
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a skillet workspace
    Init {
        /// Adopt existing .md files as .pan sources
        #[arg(long)]
        adopt: bool,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
    },
    /// Scaffold a new skill source in the current workspace
    New {
        /// Name of the skill to create
        name: String,
        /// Plugin to create the skill in (required when multiple plugins exist)
        #[arg(long)]
        plugin: Option<String>,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
    },
    /// Compile .pan source files to .md output files
    Build {
        /// Name of a single skill to compile (compiles all if omitted)
        name: Option<String>,
        /// Only build skills from this plugin
        #[arg(long)]
        plugin: Option<String>,
        /// Disable URL verification regardless of config
        #[arg(long)]
        offline: bool,
        /// Promote URL-check warnings to errors
        #[arg(long)]
        strict: bool,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
    },
    /// Show token budget for skills and agents in the workspace
    Budget {
        /// Name of a single skill or agent to show (shows all if omitted)
        name: Option<String>,
        /// Only show skills and agents from this plugin
        #[arg(long)]
        plugin: Option<String>,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
    },
    /// Verify compiled skills and agents are up-to-date with their sources
    Check {
        /// Only check skills from this plugin
        #[arg(long)]
        plugin: Option<String>,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
    },
    /// Publish plugin manifests to agent marketplace directories
    Publish {
        /// Skip the build step (use compiled outputs as-is)
        #[arg(long)]
        no_build: bool,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
        /// Distribution target (`direct` writes plugin.json/marketplace.json; `apm` writes apm.yml)
        #[arg(long, value_enum, default_value_t = PublishTargetArg::Direct)]
        target: PublishTargetArg,
    },
    /// Work with bundled skills
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
    /// Check skills for quality issues
    Lint {
        /// Name of a single skill to lint (lints all if omitted)
        name: Option<String>,
        /// Only lint skills from this plugin
        #[arg(long)]
        plugin: Option<String>,
        /// Lint only this specific source file (single-file mode for editors)
        #[arg(long, value_name = "PATH")]
        file: Option<std::path::PathBuf>,
        /// Promote warnings to errors
        #[arg(long)]
        strict: bool,
        /// Show info-level diagnostics
        #[arg(long)]
        pedantic: bool,
        /// Print per-phase timing after results
        #[arg(long)]
        verbose: bool,
        /// Output format
        #[arg(long)]
        format: Option<FormatArg>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { adopt, format } => {
            let cwd = std::env::current_dir()?;
            let json = matches!(format, Some(FormatArg::Json));
            init::run(&cwd, adopt, json)?;
        }
        Commands::New {
            name,
            plugin,
            format,
        } => {
            let cwd = std::env::current_dir()?;
            let cfg = skillet::config::SkilletConfig::load(&cwd)?;
            let json = matches!(format, Some(FormatArg::Json));
            let skills_src_dir = resolve_plugin_src_dir(&cwd, &cfg, plugin.as_deref())?;
            new::run(&skills_src_dir, &name, json)?;
        }
        Commands::Build {
            name,
            plugin,
            offline,
            strict,
            format,
        } => {
            let cwd = std::env::current_dir()?;
            let cfg = skillet::config::SkilletConfig::load(&cwd)?;
            let fmt = match format {
                Some(FormatArg::Json) => build::OutputFormat::Json,
                None => build::OutputFormat::Text,
            };
            let opts = build::BuildOptions::new_with_format(offline, strict, fmt.clone());
            build::run(&cwd, name.as_deref(), plugin.as_deref(), &opts, &cfg)?;
        }
        Commands::Check { plugin, format } => {
            let cwd = std::env::current_dir()?;
            let cfg = skillet::config::SkilletConfig::load(&cwd)?;
            let fmt = match format {
                Some(FormatArg::Json) => check::OutputFormat::Json,
                None => check::OutputFormat::Text,
            };
            let fresh = check::run(&cwd, plugin.as_deref(), fmt, &cfg)?;
            if !fresh {
                std::process::exit(1);
            }
        }
        Commands::Budget {
            name,
            plugin,
            format,
        } => {
            let cwd = std::env::current_dir()?;
            let cfg = skillet::config::SkilletConfig::load(&cwd)?;
            let fmt = match format {
                Some(FormatArg::Json) => budget::OutputFormat::Json,
                None => budget::OutputFormat::Text,
            };
            budget::run(&cwd, name.as_deref(), plugin.as_deref(), fmt, &cfg)?;
        }
        Commands::Publish {
            no_build,
            format,
            target,
        } => {
            let cwd = std::env::current_dir()?;
            let cfg = skillet::config::SkilletConfig::load(&cwd)?;
            let fmt = match format {
                Some(FormatArg::Json) => publish::OutputFormat::Json,
                None => publish::OutputFormat::Text,
            };
            let publish_target = match target {
                PublishTargetArg::Direct => publish::PublishTarget::Direct,
                PublishTargetArg::Apm => publish::PublishTarget::Apm,
            };
            publish::run(
                &cwd,
                &publish::PublishOptions {
                    no_build,
                    format: fmt,
                    target: publish_target,
                },
                &cfg,
            )?;
        }
        Commands::Skill { command } => match command {
            SkillCommands::List => crate::skill::list(),
            SkillCommands::Print { name } => crate::skill::run(&name)?,
        },
        Commands::Lint {
            name,
            plugin,
            file,
            strict,
            pedantic,
            verbose,
            format,
        } => {
            let cwd = std::env::current_dir()?;
            let cfg = skillet::config::SkilletConfig::load(&cwd)?;
            let fmt = match format {
                Some(FormatArg::Json) => lint::OutputFormat::Json,
                None => lint::OutputFormat::Text,
            };
            let mut opts = lint::LintOptions::new(strict, pedantic, fmt);
            opts.file_path = file;
            opts.verbose = verbose;
            let clean = lint::run(&cwd, name.as_deref(), plugin.as_deref(), &opts, &cfg)?;
            if !clean {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// Resolves which plugin's `src_dir` to use for `skillet new`.
///
/// If `--plugin` is specified, uses that plugin. If there is exactly one plugin,
/// uses it implicitly. Otherwise, errors asking for an explicit `--plugin` flag.
fn resolve_plugin_src_dir(
    workspace: &std::path::Path,
    cfg: &skillet::config::SkilletConfig,
    plugin_name: Option<&str>,
) -> Result<std::path::PathBuf> {
    if let Some(name) = plugin_name {
        match cfg.plugins.get(name) {
            Some(m) => return Ok(workspace.join(&m.src_dir)),
            None => bail!("plugin '{}' not found in skillet.toml", name),
        }
    }

    match cfg.plugins.len() {
        0 => bail!("no plugins defined in skillet.toml — add a [plugin.*] section"),
        1 => {
            let m = cfg.plugins.values().next().unwrap();
            Ok(workspace.join(&m.src_dir))
        }
        _ => bail!(
            "multiple plugins defined — use --plugin <name> to specify which one (available: {})",
            cfg.plugins.keys().cloned().collect::<Vec<_>>().join(", ")
        ),
    }
}
