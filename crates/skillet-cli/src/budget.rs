//! Token-budget reporting for the `skillet budget` command.

use anyhow::{Context, Result};
use serde::Serialize;
use skillet::config::SkilletConfig;
use skillet::lockfile;
use skillet::parse::parse_frontmatter;
use skillet::refs::extract_path_refs;
use skillet::tokens::count_tokens;
use skillet::workspace::{Skill, Workspace};
use std::path::Path;

// ── Public types ──────────────────────────────────────────────────────────────

/// Per-fragment token entry.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentEntry {
    pub name: String,
    pub tokens: u32,
}

/// Token budget row for a single skill.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetRow {
    pub skill: String,
    pub discovery: u32,
    pub activation: u32,
    pub transitive: u32,
    pub fragments: Vec<FragmentEntry>,
}

/// Workspace-level token totals.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetTotals {
    pub discovery: u32,
    pub activation: u32,
    pub transitive: u32,
}

/// Top-level budget report (used for JSON output).
#[derive(Debug, Clone, Serialize)]
pub struct BudgetReport {
    pub skills: Vec<BudgetRow>,
    pub totals: BudgetTotals,
}

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
    format: OutputFormat,
    config: &SkilletConfig,
) -> Result<()> {
    let ws = Workspace::resolve(workspace_path, config)?;
    let lf = lockfile::read(workspace_path)?;

    let targets: Vec<&Skill> = match skill_name {
        Some(name) => ws.skills.iter().filter(|s| s.name == name).collect(),
        None => ws.skills.iter().collect(),
    };

    let mut rows: Vec<BudgetRow> = Vec::with_capacity(targets.len());
    for skill in &targets {
        let row = compute_row(skill, &ws, &lf, &config.build.tokenizer)?;
        rows.push(row);
    }

    match format {
        OutputFormat::Text => print_text(&rows),
        OutputFormat::Json => print_json_report(&rows)?,
    }

    Ok(())
}

// ── Per-skill computation ─────────────────────────────────────────────────────

fn compute_row(
    skill: &Skill,
    ws: &Workspace,
    lockfile: &lockfile::Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    if let Some(entry) = lockfile.skills.get(&skill.name) {
        if entry.activation_tokens > 0 {
            let mut fragments = Vec::with_capacity(entry.fragments_used.len());
            let mut frag_tokens_total: u32 = 0;
            for frag_name in &entry.fragments_used {
                let tokens = lockfile
                    .fragments
                    .get(frag_name)
                    .map(|f| f.tokens)
                    .unwrap_or(0);
                frag_tokens_total += tokens;
                fragments.push(FragmentEntry {
                    name: frag_name.clone(),
                    tokens,
                });
            }
            return Ok(BudgetRow {
                skill: skill.name.clone(),
                discovery: entry.discovery_tokens,
                activation: entry.activation_tokens.saturating_sub(frag_tokens_total),
                transitive: entry.transitive_tokens,
                fragments,
            });
        }
    }

    compute_row_from_disk(skill, ws, lockfile, tokenizer)
}

fn compute_row_from_disk(
    skill: &Skill,
    ws: &Workspace,
    lockfile: &lockfile::Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    let skill_md_path = skill.skill_out_dir.join("SKILL.md");
    let compiled = std::fs::read_to_string(&skill_md_path).with_context(|| {
        format!(
            "SKILL.md not found for '{}' — run `skillet build` first",
            skill.name
        )
    })?;

    let discovery_text = match parse_frontmatter(&compiled) {
        Ok(Some(fm)) => format!(
            "{} {}",
            fm.name.unwrap_or_default(),
            fm.description.unwrap_or_default()
        ),
        _ => String::new(),
    };
    let discovery = count_tokens(&discovery_text, tokenizer);
    let compiled_tokens = count_tokens(&compiled, tokenizer);

    let source_text = std::fs::read_to_string(&skill.source_path)
        .with_context(|| format!("failed to read source '{}'", skill.source_path.display()))?;
    let ref_tokens: u32 = extract_path_refs(&source_text)
        .into_iter()
        .filter_map(|rel| {
            let path = skill.skill_dir.join(&rel);
            std::fs::read_to_string(&path)
                .ok()
                .map(|t| count_tokens(&t, tokenizer))
        })
        .sum();
    let references_tokens: u32 = skill
        .references
        .iter()
        .filter_map(|r| {
            std::fs::read_to_string(&r.absolute_path)
                .ok()
                .map(|t| count_tokens(&t, tokenizer))
        })
        .sum();
    let transitive = compiled_tokens + ref_tokens + references_tokens;

    let frag_names = lockfile
        .skills
        .get(&skill.name)
        .map(|e| e.fragments_used.as_slice())
        .unwrap_or(&[]);

    let mut fragments = Vec::with_capacity(frag_names.len());
    let mut frag_tokens_total: u32 = 0;
    for frag_name in frag_names {
        let tokens = ws.fragment_tokens.get(frag_name).copied().unwrap_or(0);
        frag_tokens_total += tokens;
        fragments.push(FragmentEntry {
            name: frag_name.clone(),
            tokens,
        });
    }

    let activation = compiled_tokens.saturating_sub(frag_tokens_total);

    Ok(BudgetRow {
        skill: skill.name.clone(),
        discovery,
        activation,
        transitive,
        fragments,
    })
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
