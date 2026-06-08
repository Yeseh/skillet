//! Token-budget computation for the `skillet budget` command.
//!
//! This module owns the domain logic for computing per-skill token budgets:
//! discovery / activation / transitive token counts and per-fragment
//! breakdowns. It prefers the recorded values in `skillet.lock` and falls
//! back to hashing/counting on-disk artefacts when the lockfile is incomplete.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::frontmatter::parse_frontmatter;
use crate::lockfile::Lockfile;
use crate::refs::extract_path_refs;
use crate::tokens::count_tokens;
use crate::workspace::{Skill, Workspace};

// ── Data types ──────────────────────────────────────────────────────────────

/// Per-fragment token entry.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentEntry {
    /// Fragment name.
    pub name: String,
    /// Tokens contributed by this fragment.
    pub tokens: u32,
}

/// Token budget row for a single skill.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetRow {
    /// Skill name.
    pub skill: String,
    /// Discovery (frontmatter name + description) tokens.
    pub discovery: u32,
    /// Activation (compiled SKILL.md minus fragments) tokens.
    pub activation: u32,
    /// Transitive (compiled + referenced files) tokens.
    pub transitive: u32,
    /// Per-fragment token breakdown.
    pub fragments: Vec<FragmentEntry>,
}

/// Workspace-level token totals.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetTotals {
    /// Sum of discovery tokens across all skills.
    pub discovery: u32,
    /// Sum of activation tokens across all skills.
    pub activation: u32,
    /// Sum of transitive tokens across all skills.
    pub transitive: u32,
}

/// Top-level budget report (used for JSON output).
#[derive(Debug, Clone, Serialize)]
pub struct BudgetReport {
    /// Per-skill rows.
    pub skills: Vec<BudgetRow>,
    /// Workspace-level totals.
    pub totals: BudgetTotals,
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Computes budget rows for the given skills (all if `skill_name` is `None`).
pub fn compute_rows(
    ws: &Workspace,
    lockfile: &Lockfile,
    skill_name: Option<&str>,
    tokenizer: &str,
) -> Result<Vec<BudgetRow>> {
    let targets: Vec<&Skill> = match skill_name {
        Some(name) => ws.skills.values().filter(|s| s.name == name).collect(),
        None => ws.skills.values().collect(),
    };

    let mut rows: Vec<BudgetRow> = Vec::with_capacity(targets.len());
    for skill in &targets {
        rows.push(compute_row(skill, ws, lockfile, tokenizer)?);
    }
    Ok(rows)
}

// ── Per-skill computation ─────────────────────────────────────────────────────

fn compute_row(
    skill: &Skill,
    ws: &Workspace,
    lockfile: &Lockfile,
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
    lockfile: &Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    let skill_md_path = skill.target_dir.join("SKILL.md");
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
            let path = skill.src_dir.join(&rel);
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
