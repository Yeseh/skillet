//! Token-budget reporting for the `skillet budget` command.
//!
//! Domain computation lives in `skillet::budget`; this module handles argument
//! plumbing and text/JSON rendering.

use anyhow::{Context, Result};
use skillet::budget::{compute_agent_rows, compute_rows, BudgetReport, BudgetRow, BudgetTotals};
use skillet::config::SkilletConfig;
use skillet::lockfile;
use skillet::workspace::Workspace;
use std::path::Path;

/// Output format for budget results.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Reports token budgets for skills and agents. `name` selects one skill or
/// agent; `plugin_name` limits the report to one plugin.
pub fn run(
    workspace_path: &Path,
    name: Option<&str>,
    plugin_name: Option<&str>,
    format: OutputFormat,
    config: &SkilletConfig,
) -> Result<()> {
    let ws = Workspace::resolve(workspace_path, config)?;
    let lf = lockfile::read(workspace_path)?;
    let tokenizer = &config.build.tokenizer;

    let mut skills = compute_rows(&ws, &lf, name, tokenizer)?;
    let mut agents = compute_agent_rows(&ws, &lf, name, tokenizer)?;

    if let Some(plugin_name) = plugin_name {
        skills.retain(|row| {
            ws.skills
                .get(&row.name)
                .is_some_and(|s| s.plugin == plugin_name)
        });
        agents.retain(|row| {
            ws.agents
                .get(&row.name)
                .is_some_and(|a| a.plugin == plugin_name)
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    agents.sort_by(|a, b| a.name.cmp(&b.name));

    match format {
        OutputFormat::Text => print_text(&skills, &agents),
        OutputFormat::Json => print_json_report(&skills, &agents)?,
    }

    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn fragments_text(row: &BudgetRow) -> String {
    row.fragments
        .iter()
        .map(|f| format!("{}({})", f.name, f.tokens))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_text(skills: &[BudgetRow], agents: &[BudgetRow]) {
    if skills.is_empty() && agents.is_empty() {
        println!("No skills or agents found.");
        return;
    }

    let all = || skills.iter().chain(agents);
    let w_name = all()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .max("Skill".len());
    let w_disc = "Discovery".len();
    let w_act = "Activation".len();
    let w_trans = "Transitive".len();
    let w_frags = all()
        .map(|r| fragments_text(r).len())
        .max()
        .unwrap_or(0)
        .max("Fragments".len());
    let sep_width = w_name + 2 + w_disc + 2 + w_act + 2 + w_trans + 2 + w_frags;

    let print_section = |title: &str, rows: &[BudgetRow]| {
        if rows.is_empty() {
            return;
        }
        println!(
            "{:<w_name$}  {:>w_disc$}  {:>w_act$}  {:>w_trans$}  {:<w_frags$}",
            title, "Discovery", "Activation", "Transitive", "Fragments",
        );
        println!("{}", "─".repeat(sep_width));
        for row in rows {
            println!(
                "{:<w_name$}  {:>w_disc$}  {:>w_act$}  {:>w_trans$}  {}",
                row.name,
                row.discovery,
                row.activation,
                row.transitive,
                fragments_text(row),
            );
        }
        println!("{}", "─".repeat(sep_width));
    };

    print_section("Skill", skills);
    if !skills.is_empty() && !agents.is_empty() {
        println!();
    }
    print_section("Agent", agents);

    let total_disc: u32 = all().map(|r| r.discovery).sum();
    let total_act: u32 = all().map(|r| r.activation).sum();
    println!(
        "{:<w_name$}  {:>w_disc$}  {:>w_act$}",
        "TOTAL", total_disc, total_act,
    );
}

fn print_json_report(skills: &[BudgetRow], agents: &[BudgetRow]) -> Result<()> {
    let all = || skills.iter().chain(agents);
    let totals = BudgetTotals {
        discovery: all().map(|r| r.discovery).sum(),
        activation: all().map(|r| r.activation).sum(),
        transitive: all().map(|r| r.transitive).sum(),
    };
    let report = BudgetReport {
        skills: skills.to_vec(),
        agents: agents.to_vec(),
        totals,
    };
    let json =
        serde_json::to_string_pretty(&report).context("failed to serialise budget as JSON")?;
    println!("{}", json);
    Ok(())
}
