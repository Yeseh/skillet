//! Freshness verification CLI: compares workspace state against `skillet.lock`.
//!
//! `skillet check` is designed as a fast CI gate — it never recompiles; it
//! just hashes source files and on-disk `SKILL.md` and agent files and compares them
//! against the recorded hashes in the lockfile.
//!
//! The verdict computation lives in `skillet::freshness`; this module handles
//! the I/O precondition, output formatting, and the process exit signal.

use anyhow::{Context, Result};
use skillet::config::SkilletConfig;
use skillet::freshness::{self, ArtefactResult, CheckReport};
use skillet::lockfile;
use skillet::workspace::Workspace;
use std::path::Path;

/// How results are rendered.
#[derive(Clone, Copy, Default)]
pub enum OutputFormat {
    /// Default text output.
    #[default]
    Text,
    /// Machine-parseable JSON output.
    Json,
}

/// Runs freshness checks for all skills and agents in the workspace.
pub fn run(
    workspace_path: &Path,
    _plugin_name: Option<&str>,
    format: OutputFormat,
    config: &SkilletConfig,
) -> Result<bool> {
    let ws = Workspace::resolve(workspace_path, config)?;

    let lock_path = workspace_path.join("skillet.lock");
    if !lock_path.exists() {
        anyhow::bail!("skillet.lock not found — run `skillet build` to generate it");
    }
    let lockfile = lockfile::read(workspace_path)
        .with_context(|| format!("failed to read {}", lock_path.display()))?;

    let report = freshness::verify(&ws, &lockfile);

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Text => render(&report),
    }

    Ok(report.fresh)
}

fn render(report: &CheckReport) {
    let stale_skills: Vec<&ArtefactResult> = report.skills.iter().filter(|r| !r.fresh).collect();
    let stale_agents: Vec<&ArtefactResult> = report.agents.iter().filter(|r| !r.fresh).collect();

    if stale_skills.is_empty() && stale_agents.is_empty() {
        println!(
            "✓ all {} up-to-date",
            counted(report.skills.len(), report.agents.len())
        );
        return;
    }

    for skill in &stale_skills {
        for reason in &skill.reasons {
            println!("✗ {}: {}", skill.name, reason);
        }
    }
    for agent in &stale_agents {
        for reason in &agent.reasons {
            println!("✗ agent {}: {}", agent.name, reason);
        }
    }
    println!(
        "\n{} stale",
        counted(stale_skills.len(), stale_agents.len())
    );
}

/// Formats a count of skills and agents, e.g. "2 skills and 1 agent".
/// The agent part is left out when there are no agents.
fn counted(skills: usize, agents: usize) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    if agents == 0 {
        plural(skills, "skill")
    } else {
        format!(
            "{} and {}",
            plural(skills, "skill"),
            plural(agents, "agent")
        )
    }
}
