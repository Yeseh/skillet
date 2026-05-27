//! Token-budget reporting for the `skillet budget` command.

use anyhow::{Context, Result};
use serde::Serialize;
use skillet::config::SkilletConfig;
use skillet::lockfile;
use skillet::parse::parse_frontmatter;
use skillet::refs::extract_path_refs;
use skillet::tokens::count_tokens;
use skillet::workspace::{self, SkillSource};
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
    let skills_src_dir = workspace_path.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace_path.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace_path.join(&config.workspace.fragments_dir);

    let all_sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;
    let lf = lockfile::read(workspace_path)?;

    let targets: Vec<_> = match skill_name {
        Some(name) => all_sources.iter().filter(|s| s.name == name).collect(),
        None => all_sources.iter().collect(),
    };

    let mut rows: Vec<BudgetRow> = Vec::with_capacity(targets.len());
    for source in &targets {
        let row = compute_row(source, &fragments_dir, &lf, &config.build.tokenizer)?;
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
    source: &SkillSource,
    fragments_dir: &Path,
    lockfile: &lockfile::Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    if let Some(entry) = lockfile.skills.get(&source.name) {
        if entry.activation_tokens > 0 {
            let mut fragments = Vec::with_capacity(entry.fragments_used.len());
            for frag_name in &entry.fragments_used {
                let tokens = lockfile
                    .fragments
                    .get(frag_name)
                    .map(|f| f.tokens)
                    .unwrap_or(0);
                fragments.push(FragmentEntry {
                    name: frag_name.clone(),
                    tokens,
                });
            }
            return Ok(BudgetRow {
                skill: source.name.clone(),
                discovery: entry.discovery_tokens,
                activation: entry.activation_tokens,
                transitive: entry.transitive_tokens,
                fragments,
            });
        }
    }

    compute_row_from_disk(source, fragments_dir, lockfile, tokenizer)
}

fn compute_row_from_disk(
    source: &SkillSource,
    fragments_dir: &Path,
    lockfile: &lockfile::Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    let skill_md_path = source.skill_out_dir.join("SKILL.md");
    let compiled = std::fs::read_to_string(&skill_md_path).with_context(|| {
        format!(
            "SKILL.md not found for '{}' — run `skillet build` first",
            source.name
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
    let activation = count_tokens(&compiled, tokenizer);

    let source_text = std::fs::read_to_string(&source.source_path)
        .with_context(|| format!("failed to read source '{}'", source.source_path.display()))?;
    let ref_tokens: u32 = extract_path_refs(&source_text)
        .into_iter()
        .filter_map(|rel| {
            let path = source.skill_dir.join(&rel);
            std::fs::read_to_string(&path)
                .ok()
                .map(|t| count_tokens(&t, tokenizer))
        })
        .sum();
    let transitive = activation + ref_tokens;

    let frag_names = lockfile
        .skills
        .get(&source.name)
        .map(|e| e.fragments_used.as_slice())
        .unwrap_or(&[]);

    let mut fragments = Vec::with_capacity(frag_names.len());
    for frag_name in frag_names {
        let frag_path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        let tokens = if let Ok(text) = std::fs::read_to_string(&frag_path) {
            count_tokens(&text, tokenizer)
        } else {
            0
        };
        fragments.push(FragmentEntry {
            name: frag_name.clone(),
            tokens,
        });
    }

    Ok(BudgetRow {
        skill: source.name.clone(),
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
