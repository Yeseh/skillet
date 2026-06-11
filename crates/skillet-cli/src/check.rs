//! Freshness verification CLI: compares workspace state against `skillet.lock`.
//!
//! `skillet check` is designed as a fast CI gate — it never recompiles; it
//! just hashes source files and on-disk `SKILL.md` files and compares them
//! against the recorded hashes in the lockfile.
//!
//! The verdict computation lives in `skillet::freshness`; this module handles
//! the I/O precondition, output formatting, and the process exit signal.

use anyhow::{Context, Result};
use skillet::config::SkilletConfig;
use skillet::freshness::{self, CheckReport, SkillResult};
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

/// Runs freshness checks for all skills in the workspace.
pub fn run(
    workspace_path: &Path,
    _module_name: Option<&str>,
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
    let stale: Vec<&SkillResult> = report.skills.iter().filter(|r| !r.fresh).collect();

    if stale.is_empty() {
        let n = report.skills.len();
        println!(
            "✓ all {} skill{} up-to-date",
            n,
            if n == 1 { " is" } else { "s are" }
        );
        return;
    }

    for skill in &stale {
        for reason in &skill.reasons {
            println!("✗ {}: {}", skill.name, reason);
        }
    }
    println!(
        "\n{} skill{} stale",
        stale.len(),
        if stale.len() == 1 { " is" } else { "s are" }
    );
}
