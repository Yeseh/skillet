//! Token-budget computation for the `skillet budget` command.
//!
//! This module owns the domain logic for computing per-skill and per-agent
//! token budgets:
//! discovery / activation / transitive token counts and per-fragment
//! breakdowns. It prefers the recorded values in `skillet.lock` and falls
//! back to hashing/counting on-disk artefacts when the lockfile is incomplete.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::frontmatter::parse_frontmatter;
use crate::lockfile::{ArtefactEntry, Lockfile};
use crate::tokens::count_tokens;
use crate::workspace::{Agent, Skill, Workspace};

// ── Data types ──────────────────────────────────────────────────────────────

/// Per-fragment token entry.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentEntry {
    /// Fragment name.
    pub name: String,
    /// Tokens contributed by this fragment.
    pub tokens: u32,
}

/// Token budget row for a single skill or agent.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetRow {
    /// Skill or agent name.
    pub name: String,
    /// Discovery (frontmatter name + description) tokens.
    pub discovery: u32,
    /// Activation (compiled SKILL.md minus fragments) tokens.
    pub activation: u32,
    /// Transitive (compiled + referenced files) tokens. Equal to the full
    /// compiled size for agents, which have no referenced files.
    pub transitive: u32,
    /// Per-fragment token breakdown.
    pub fragments: Vec<FragmentEntry>,
}

/// Workspace-level token totals.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetTotals {
    /// Sum of discovery tokens across all skills and agents.
    pub discovery: u32,
    /// Sum of activation tokens across all skills and agents.
    pub activation: u32,
    /// Sum of transitive tokens across all skills and agents.
    pub transitive: u32,
}

/// Top-level budget report (used for JSON output).
#[derive(Debug, Clone, Serialize)]
pub struct BudgetReport {
    /// Per-skill rows.
    pub skills: Vec<BudgetRow>,
    /// Per-agent rows.
    pub agents: Vec<BudgetRow>,
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

/// Computes budget rows for the given agents (all if `agent_name` is `None`).
pub fn compute_agent_rows(
    ws: &Workspace,
    lockfile: &Lockfile,
    agent_name: Option<&str>,
    tokenizer: &str,
) -> Result<Vec<BudgetRow>> {
    ws.agents
        .values()
        .filter(|a| agent_name.is_none_or(|n| a.name == n))
        .map(|agent| match lockfile.agents.get(&agent.name) {
            Some(entry) if entry.activation_tokens > 0 => {
                Ok(row_from_entry(&agent.name, entry, lockfile))
            }
            _ => compute_agent_row_from_disk(agent, ws, lockfile, tokenizer),
        })
        .collect()
}

/// Builds a row from recorded lockfile values. Activation excludes the tokens
/// of the fragments the artefact uses; they are listed separately.
fn row_from_entry(name: &str, entry: &ArtefactEntry, lockfile: &Lockfile) -> BudgetRow {
    let fragments: Vec<FragmentEntry> = entry
        .fragments_used
        .iter()
        .map(|frag_name| FragmentEntry {
            name: frag_name.clone(),
            tokens: lockfile
                .fragments
                .get(frag_name)
                .map(|f| f.tokens)
                .unwrap_or(0),
        })
        .collect();
    let frag_tokens_total: u32 = fragments.iter().map(|f| f.tokens).sum();

    BudgetRow {
        name: name.to_string(),
        discovery: entry.discovery_tokens,
        activation: entry.activation_tokens.saturating_sub(frag_tokens_total),
        transitive: entry.transitive_tokens,
        fragments,
    }
}

fn compute_agent_row_from_disk(
    agent: &Agent,
    ws: &Workspace,
    lockfile: &Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    let path = agent
        .out_dir
        .join(crate::agent::canonical_path(&agent.name));
    let compiled = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "agent '{}' not found at {} — run `skillet build` first",
            agent.name,
            path.display()
        )
    })?;

    let frag_names = lockfile
        .agents
        .get(&agent.name)
        .map(|e| e.fragments_used.as_slice())
        .unwrap_or(&[]);

    Ok(row_from_compiled(
        &agent.name,
        &compiled,
        0,
        frag_names,
        ws,
        tokenizer,
    ))
}

/// Builds a row by counting tokens in compiled text. `extra_transitive` is the
/// token count of files the artefact makes the agent read (skill references).
fn row_from_compiled(
    name: &str,
    compiled: &str,
    extra_transitive: u32,
    frag_names: &[String],
    ws: &Workspace,
    tokenizer: &str,
) -> BudgetRow {
    let discovery_text = match parse_frontmatter(compiled) {
        Ok(Some(fm)) => format!(
            "{} {}",
            fm.name.unwrap_or_default(),
            fm.description.unwrap_or_default()
        ),
        _ => String::new(),
    };
    let compiled_tokens = count_tokens(compiled, tokenizer);

    let fragments: Vec<FragmentEntry> = frag_names
        .iter()
        .map(|frag_name| FragmentEntry {
            name: frag_name.clone(),
            tokens: ws.fragment_tokens.get(frag_name).copied().unwrap_or(0),
        })
        .collect();
    let frag_tokens_total: u32 = fragments.iter().map(|f| f.tokens).sum();

    BudgetRow {
        name: name.to_string(),
        discovery: count_tokens(&discovery_text, tokenizer),
        activation: compiled_tokens.saturating_sub(frag_tokens_total),
        transitive: compiled_tokens + extra_transitive,
        fragments,
    }
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
            return Ok(row_from_entry(&skill.name, entry, lockfile));
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

    // Every `.pan`/`.md` file under `references/` is bundled into the built skill,
    // and `ref::name` directives can only target those files, so the full set of
    // reference files is the transitive cost — counting `ref::` targets separately
    // would double-count the ones that are referenced.
    let references_tokens: u32 = skill
        .references
        .iter()
        .filter_map(|r| {
            std::fs::read_to_string(&r.absolute_path)
                .ok()
                .map(|t| count_tokens(&t, tokenizer))
        })
        .sum();

    let frag_names = lockfile
        .skills
        .get(&skill.name)
        .map(|e| e.fragments_used.as_slice())
        .unwrap_or(&[]);

    Ok(row_from_compiled(
        &skill.name,
        &compiled,
        references_tokens,
        frag_names,
        ws,
        tokenizer,
    ))
}
