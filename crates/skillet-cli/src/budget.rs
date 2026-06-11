//! Token-budget reporting for the `skillet budget` command.
//!
//! Domain computation lives in `skillet::budget`; this module handles argument
//! plumbing and text/JSON rendering.

use anyhow::{Context, Result};
use skillet::budget::{compute_rows, BudgetReport, BudgetRow, BudgetTotals};
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

pub fn run(
    workspace_path: &Path,
    skill_name: Option<&str>,
    module_name: Option<&str>,
    format: OutputFormat,
    config: &SkilletConfig,
) -> Result<()> {
    let ws = Workspace::resolve(workspace_path, config)?;
    let lf = lockfile::read(workspace_path)?;

    let mut rows = compute_rows(&ws, &lf, skill_name, &config.build.tokenizer)?;

    if let Some(mod_name) = module_name {
        rows.retain(|row| {
            ws.skills
                .get(&row.skill)
                .map(|s| s.module == mod_name)
                .unwrap_or(false)
        });
    }

    match format {
        OutputFormat::Text => print_text(&rows),
        OutputFormat::Json => print_json_report(&rows)?,
    }

    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn print_text(rows: &[BudgetRow]) {
    if rows.is_empty() {
        println!("No skills found.");
        return;
    }

    let w_skill = rows.iter().map(|r| r.skill.len()).max().unwrap_or(5).max(5);
    let headers = [
        "Skill",
        "Discovery",
        "Activation",
        "Transitive",
        "Fragments",
    ];
    let w_disc = "Discovery".len();
    let w_act = "Activation".len();
    let w_trans = "Transitive".len();
    let w_frags = rows
        .iter()
        .map(|r| {
            if r.fragments.is_empty() {
                0
            } else {
                r.fragments
                    .iter()
                    .map(|f| format!("{}({})", f.name, f.tokens).len())
                    .sum::<usize>()
                    + (r.fragments.len() - 1) * 2
            }
        })
        .max()
        .unwrap_or(0)
        .max("Fragments".len());

    println!(
        "{:<w_skill$}  {:>w_disc$}  {:>w_act$}  {:>w_trans$}  {:<w_frags$}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        headers[4],
        w_skill = w_skill,
        w_disc = w_disc,
        w_act = w_act,
        w_trans = w_trans,
        w_frags = w_frags,
    );

    let sep_width = w_skill + 2 + w_disc + 2 + w_act + 2 + w_trans + 2 + w_frags;
    println!("{}", "─".repeat(sep_width));

    let mut total_disc: u32 = 0;
    let mut total_act: u32 = 0;

    for row in rows {
        let frag_str = if row.fragments.is_empty() {
            String::new()
        } else {
            row.fragments
                .iter()
                .map(|f| format!("{}({})", f.name, f.tokens))
                .collect::<Vec<_>>()
                .join(", ")
        };

        println!(
            "{:<w_skill$}  {:>w_disc$}  {:>w_act$}  {:>w_trans$}  {}",
            row.skill,
            row.discovery,
            row.activation,
            row.transitive,
            frag_str,
            w_skill = w_skill,
            w_disc = w_disc,
            w_act = w_act,
            w_trans = w_trans,
        );

        total_disc += row.discovery;
        total_act += row.activation;
    }

    println!("{}", "─".repeat(sep_width));
    println!(
        "{:<w_skill$}  {:>w_disc$}  {:>w_act$}",
        "TOTAL",
        total_disc,
        total_act,
        w_skill = w_skill,
        w_disc = w_disc,
        w_act = w_act,
    );
}

fn print_json_report(rows: &[BudgetRow]) -> Result<()> {
    let totals = BudgetTotals {
        discovery: rows.iter().map(|r| r.discovery).sum(),
        activation: rows.iter().map(|r| r.activation).sum(),
        transitive: rows.iter().map(|r| r.transitive).sum(),
    };
    let report = BudgetReport {
        skills: rows.to_vec(),
        totals,
    };
    let json =
        serde_json::to_string_pretty(&report).context("failed to serialise budget as JSON")?;
    println!("{}", json);
    Ok(())
}
