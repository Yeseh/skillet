//! Lint engine for skill quality validation.
//!
//! Lint is an aggregator over the existing pipeline stages rather than a
//! reimplementation of them.  For every target skill it runs, in order:
//!
//! 1. [`check_source_file`](crate::compiler::check::check_source_file) —
//!    referential integrity (broken `ref::`/`skill::`/`var::`/`env::`/`cmd::`
//!    and fragment includes).
//! 2. [`compile`](crate::compiler::compile::compile) — produces the compiled
//!    text plus `fragments_used`, `activation_tokens`, and `discovery_tokens`.
//!
//! Workspace freshness (`stale-build`) is delegated to
//! [`freshness::verify`](crate::freshness::verify) — the same logic behind
//! `skillet check`.  Only the genuinely lint-specific concerns
//! (frontmatter validity, untyped-backtick hints, size budgets, unused
//! fragments, cross-skill duplication) live in the `rules` submodule.
//!
//! Agent sources go through the same `check` and `compile` stages, then
//! through [`AgentDefinition::parse`]; each [`AgentError`](crate::agent::AgentError)
//! code becomes an `agent-{code}` rule.

use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::agent::{AgentDefinition, AgentError};
use crate::compiler::check::{check_source_file, CheckDiag, CheckKind};
use crate::compiler::compile::{body_start_offset, compile, CompileOutput};
use crate::compiler::parse::{Node, PanParse};
use crate::compiler::PanSource;
use crate::config::SkilletConfig;
use crate::freshness;
use crate::lockfile::Lockfile;
use crate::refs::ParsedRefs;
use crate::workspace::{Agent, Skill, Workspace};

/// Individual lint rule implementations.
pub mod rules;

// ── Compiled skill ──────────────────────────────────────────────────────────────

/// One skill carried through `check` and `compile`, ready for the lint rules.
///
/// This is the lint engine's view of a skill: the raw source plus the outputs
/// of the shared pipeline stages.  No hashing, tokenizing, or ref extraction is
/// duplicated here — those come from [`compile`] and [`check_source_file`].
pub struct CompiledSkill {
    /// Skill name (directory name).
    pub name: String,
    /// Plugin this skill belongs to.
    pub plugin: String,
    /// Absolute path to the `.pan` source file.
    pub source_path: PathBuf,
    /// Raw source content (empty if unreadable).
    pub raw: String,
    /// Referential-integrity diagnostics from the check stage.
    pub check_diags: Vec<CheckDiag>,
    /// Output of the compile stage.
    pub output: CompileOutput,
    /// All refs extracted from the raw source (typed, links, untyped suspects).
    pub parsed_refs: ParsedRefs,
}

fn compile_skill(skill: &Skill, ws: &Workspace) -> CompiledSkill {
    let raw = std::fs::read_to_string(&skill.source_path).unwrap_or_default();
    let source = PanSource::new(raw.clone());

    let check_diags = check_source_file(ws, &source, &skill.references);
    let output = compile(ws, &source, &skill.references);
    let skill_names: Vec<&str> = ws.skills.keys().map(|s| s.as_str()).collect();
    let parsed_refs = ParsedRefs::extract(&raw, &skill_names);

    CompiledSkill {
        name: skill.name.clone(),
        plugin: skill.plugin.clone(),
        source_path: skill.source_path.clone(),
        raw,
        check_diags,
        output,
        parsed_refs,
    }
}

// ── Public types ──────────────────────────────────────────────────────────────

/// Diagnostic severity level.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational — shown only with `--pedantic`.
    Info,
    /// Warning — build succeeds; promoted to error with `--strict`.
    Warning,
    /// Error — build fails; `skillet lint` exits non-zero.
    Error,
}

/// A secondary source location associated with a [`Diagnostic`].
#[derive(Debug, Clone, Serialize)]
pub struct RelatedLocation {
    /// File path of the related location.
    pub path: String,
    /// 1-based line number (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based column number (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
    /// Human-readable description of this related location.
    pub message: String,
}

/// A single lint finding.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    /// Short rule identifier (e.g. `"stale-path-ref"`).
    pub rule: String,
    /// Severity of the finding.
    pub severity: Severity,
    /// Skill name this applies to, or `"<workspace>"` for workspace-level rules.
    pub skill: String,
    /// Description of the problem.
    pub message: String,
    /// File path where the issue was found (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 1-based line number where the issue was found (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based column number where the issue was found (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,

    /// The duplicated passage text (duplication rule only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicated_text: Option<String>,
    /// Skill names that share the duplicated passage (duplication rule only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected_skills: Option<Vec<String>>,
    /// Secondary source locations related to this diagnostic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<RelatedLocation>>,
}

/// Output format for lint results.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Default coloured text output.
    #[default]
    Text,
    /// Machine-parseable JSON array.
    Json,
    /// Suppress all output (useful for benchmarks and programmatic use).
    Silent,
}

/// Options controlling lint behaviour.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct LintOptions {
    /// Promote all warnings to errors.
    pub strict: bool,
    /// Show info-level diagnostics.
    pub pedantic: bool,
    /// Output format.
    pub format: OutputFormat,
    /// Single-skill mode: lint only this skill, skipping workspace rules.
    pub skill: Option<String>,
    /// Single-plugin mode: lint only skills in this plugin.
    pub plugin: Option<String>,
    /// Single-file mode: lint only this `.pan` file, skipping workspace rules.
    ///
    /// Used by editor integrations (e.g. `skillet lint --file <path>`).
    pub file_path: Option<PathBuf>,
    /// Print elapsed time per phase and total to stdout after results.
    pub verbose: bool,
}

impl LintOptions {
    /// Creates a new `LintOptions` with the legacy three-parameter set.
    pub fn new(strict: bool, pedantic: bool, format: OutputFormat) -> Self {
        Self {
            strict,
            pedantic,
            format,
            skill: None,
            plugin: None,
            file_path: None,
            verbose: false,
        }
    }
}

/// Result of a [`lint`] run.
#[derive(Debug, Default)]
pub struct LintOutput {
    /// All diagnostics produced, before severity promotion or filtering.
    pub diagnostics: Vec<Diagnostic>,
    /// Recomputed MinHash signatures `(skill_name, signature)` that the caller
    /// should write back to the lockfile.
    pub updated_minhash: Vec<(String, Vec<u64>)>,
}

// ── Entry point ─────────────────────────────────────────────────────────────────

/// Runs all lint rules over the resolved [`Workspace`].
///
/// Selects target skills from `opts` (single file, single skill, or the whole
/// workspace), runs `check` + `compile` for each, then runs the lint-specific
/// rules.  Severity promotion (`--strict`), info filtering (`--pedantic`),
/// disabled-rule filtering, rendering, and lockfile writeback are the caller's
/// responsibility.
pub fn lint(
    ws: &Workspace,
    lockfile: &Lockfile,
    config: &SkilletConfig,
    opts: &LintOptions,
) -> LintOutput {
    let abs_file = opts.file_path.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            ws.root.join(path)
        }
    });

    let targets: Vec<&Skill> = match (&abs_file, &opts.skill) {
        (Some(abs), _) => ws
            .skills
            .values()
            .filter(|s| &s.source_path == abs)
            .collect(),
        (None, Some(name)) => ws.skills.values().filter(|s| &s.name == name).collect(),
        (None, None) => {
            if let Some(plugin_name) = &opts.plugin {
                ws.skills
                    .values()
                    .filter(|s| &s.plugin == plugin_name)
                    .collect()
            } else {
                ws.skills.values().collect()
            }
        }
    };

    let agent_targets: Vec<&Agent> = ws
        .agents
        .values()
        .filter(|a| match (&abs_file, &opts.skill, &opts.plugin) {
            (Some(abs), _, _) => &a.source_path == abs,
            (None, Some(name), _) => &a.name == name,
            (None, None, Some(plugin)) => &a.plugin == plugin,
            (None, None, None) => true,
        })
        .collect();

    let run_workspace_rules = opts.file_path.is_none() && opts.skill.is_none();
    let target_names: HashSet<&str> = targets.iter().map(|s| s.name.as_str()).collect();

    let compiled: Vec<CompiledSkill> = targets.par_iter().map(|s| compile_skill(s, ws)).collect();

    // Per-skill rules and workspace rules run concurrently.
    let (mut diagnostics, (workspace_diags, updated_minhash)) = rayon::join(
        || -> Vec<Diagnostic> {
            let skill_names: Vec<&str> = ws.skills.keys().map(|s| s.as_str()).collect();
            compiled
                .par_iter()
                .flat_map(|cs| lint_skill_rules(cs, ws, config, &skill_names))
                .collect()
        },
        || -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
            if run_workspace_rules {
                lint_workspace_rules(&compiled, ws, config, lockfile)
            } else {
                (vec![], vec![])
            }
        },
    );

    diagnostics.extend(workspace_diags);
    diagnostics.extend(agent_targets.iter().flat_map(|a| lint_agent(a, ws)));

    // `stale-build` reuses the freshness verifier (the `skillet check` logic).
    let agent_names: HashSet<&str> = agent_targets.iter().map(|a| a.name.as_str()).collect();
    diagnostics.extend(stale_build_diags(
        ws,
        lockfile,
        &target_names,
        &agent_names,
        run_workspace_rules,
    ));

    LintOutput {
        diagnostics,
        updated_minhash,
    }
}

fn lint_skill_rules(
    cs: &CompiledSkill,
    ws: &Workspace,
    config: &SkilletConfig,
    skill_names: &[&str],
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let file_path = cs.source_path.to_string_lossy().to_string();

    diags.extend(rules::invalid_frontmatter::check(
        &cs.name, &file_path, &cs.raw,
    ));

    // Referential integrity comes straight from the check stage.
    for d in &cs.check_diags {
        diags.push(diag_from_check(&cs.name, &file_path, d));
    }

    diags.extend(rules::untyped_backtick::check(
        &cs.name,
        &file_path,
        &cs.raw,
        skill_names,
    ));
    diags.extend(rules::oversized::check_skill(cs, config));
    diags.extend(rules::oversized::check_description(cs, config));

    // cross-plugin-fragment-ref: warn when a plugin skill uses a workspace-global fragment.
    if !ws.global_fragment_names.is_empty() {
        let plugin_local = ws.plugin_fragment_names.get(&cs.plugin);
        for frag_name in &cs.output.fragments_used {
            if ws.global_fragment_names.contains(frag_name) {
                let is_also_local = plugin_local
                    .map(|s| s.contains(frag_name.as_str()))
                    .unwrap_or(false);
                if !is_also_local {
                    let loc = find_fragment_include_location(&cs.raw, frag_name);
                    diags.push(diag_located(
                        Severity::Warning,
                        &cs.name,
                        "cross-plugin-fragment-ref",
                        format!(
                            "fragment '{{{{> {frag_name} }}}}' is workspace-global and will not \
                             be available when plugin '{}' is published standalone",
                            cs.plugin
                        ),
                        Some(file_path.clone()),
                        loc.map(|(line, _)| line).or(Some(1)),
                        loc.map(|(_, col)| col),
                    ));
                }
            }
        }
    }

    diags.extend(cross_plugin_skill_refs(
        &cs.name,
        &cs.plugin,
        &cs.parsed_refs,
        ws,
        &file_path,
    ));

    diags
}

/// cross-plugin-skill-ref: error when a `skill::` ref targets a skill in a different plugin.
fn cross_plugin_skill_refs(
    name: &str,
    plugin: &str,
    parsed_refs: &ParsedRefs,
    ws: &Workspace,
    file_path: &str,
) -> Vec<Diagnostic> {
    use crate::refs::RefKind;

    parsed_refs
        .typed
        .iter()
        .filter(|r| r.kind == RefKind::Skill)
        .filter_map(|typed_ref| {
            let target = ws.skills.get(&typed_ref.value)?;
            (target.plugin != plugin).then(|| {
                diag_located(
                    Severity::Error,
                    name,
                    "cross-plugin-skill-ref",
                    format!(
                        "`skill::{}` is in plugin '{}' and cannot be referenced from plugin '{}'",
                        typed_ref.value, target.plugin, plugin
                    ),
                    Some(file_path.to_string()),
                    Some(typed_ref.line),
                    Some(typed_ref.col),
                )
            })
        })
        .collect()
}

/// Lints one agent source: referential integrity, the agent definition rules
/// (`agent-{code}`), Claude Code field support, and cross-plugin skill refs.
fn lint_agent(agent: &Agent, ws: &Workspace) -> Vec<Diagnostic> {
    let file_path = agent.source_path.to_string_lossy().to_string();
    let raw = std::fs::read_to_string(&agent.source_path).unwrap_or_default();
    let source = PanSource::new(raw.clone());

    // An agent has no `references/` directory, so every `ref::` is reported.
    let mut diags: Vec<Diagnostic> = check_source_file(ws, &source, &[])
        .iter()
        .map(|d| diag_from_check(&agent.name, &file_path, d))
        .collect();

    let output = compile(ws, &source, &[]);
    match AgentDefinition::parse(&output.text, &agent.name) {
        Err(e) => {
            let line = match &e {
                AgentError::InvalidName(_) | AgentError::NameMismatch { .. } => {
                    frontmatter_field_line(&raw, "name").or(Some(1))
                }
                AgentError::MissingDescription => {
                    frontmatter_field_line(&raw, "description").or(Some(1))
                }
                AgentError::BodyTooLong { .. } => Some(body_start_line(&raw)),
                AgentError::MissingFrontmatter
                | AgentError::InvalidFrontmatter(_)
                | AgentError::MissingName => Some(1),
            };
            diags.push(diag_located(
                Severity::Error,
                &agent.name,
                &format!("agent-{}", e.code()),
                e.to_string(),
                Some(file_path.clone()),
                line,
                None,
            ));
        }
        Ok(def) => {
            for field in def.claude_ignored_fields() {
                diags.push(diag_located(
                    Severity::Warning,
                    &agent.name,
                    "agent-field-ignored-by-claude",
                    format!("Claude Code ignores '{field}' for agents loaded from a plugin"),
                    Some(file_path.clone()),
                    frontmatter_field_line(&raw, field).or(Some(1)),
                    None,
                ));
            }
        }
    }

    let skill_names: Vec<&str> = ws.skills.keys().map(|s| s.as_str()).collect();
    let parsed_refs = ParsedRefs::extract(&raw, &skill_names);
    diags.extend(cross_plugin_skill_refs(
        &agent.name,
        &agent.plugin,
        &parsed_refs,
        ws,
        &file_path,
    ));

    diags
}

fn lint_workspace_rules(
    compiled: &[CompiledSkill],
    ws: &Workspace,
    config: &SkilletConfig,
    lockfile: &Lockfile,
) -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
    let mut diags = Vec::new();
    diags.extend(rules::unused_fragment::check(compiled, ws));
    diags.extend(rules::unused_reference::check(compiled, ws));
    diags.extend(rules::oversized::check_fragments(ws, config));
    let (dup_diags, updated_sigs) = rules::duplication::check(compiled, lockfile);
    diags.extend(dup_diags);
    (diags, updated_sigs)
}

/// Maps a referential-integrity [`CheckDiag`] onto a lint [`Diagnostic`],
/// deriving the rule slug from its [`CheckKind`].
fn diag_from_check(skill: &str, path: &str, d: &CheckDiag) -> Diagnostic {
    use crate::compiler::check::Severity as CheckSeverity;

    let rule = match d.kind {
        CheckKind::Fragment => "stale-fragment-ref",
        CheckKind::PathRef => "stale-path-ref",
        CheckKind::Command => "stale-command-ref",
        CheckKind::Skill => "stale-skill-ref",
        CheckKind::Var => "stale-var-ref",
        CheckKind::Env => "stale-env-ref",
    };
    let severity = match d.severity {
        CheckSeverity::Error => Severity::Error,
        CheckSeverity::Warning => Severity::Warning,
    };
    diag_located(
        severity,
        skill,
        rule,
        d.message.clone(),
        Some(path.to_string()),
        Some(d.line),
        Some(d.col),
    )
}

/// Produces `stale-build` diagnostics by delegating to [`freshness::verify`].
fn stale_build_diags(
    ws: &Workspace,
    lockfile: &Lockfile,
    target_names: &HashSet<&str>,
    agent_names: &HashSet<&str>,
    run_workspace_rules: bool,
) -> Vec<Diagnostic> {
    let report = freshness::verify(ws, lockfile);

    let skills = report
        .skills
        .iter()
        .filter(|r| run_workspace_rules || target_names.contains(r.name.as_str()));
    let agents = report
        .agents
        .iter()
        .filter(|r| run_workspace_rules || agent_names.contains(r.name.as_str()));

    skills
        .chain(agents)
        .filter(|r| !r.fresh)
        .flat_map(|r| {
            let path = ws
                .skills
                .get(&r.name)
                .map(|s| s.source_path.to_string_lossy().to_string())
                .or_else(|| {
                    ws.agents
                        .get(&r.name)
                        .map(|a| a.source_path.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| ws.root.join("skillet.lock").to_string_lossy().to_string());
            r.reasons.iter().map(move |reason| {
                diag_located(
                    Severity::Error,
                    &r.name,
                    "stale-build",
                    reason.clone(),
                    Some(path.clone()),
                    Some(1),
                    None,
                )
            })
        })
        .collect()
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Finds the 1-based line number of a top-level YAML key inside frontmatter.
pub(crate) fn frontmatter_field_line(raw: &str, field: &str) -> Option<u32> {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for (idx, line) in lines.enumerate() {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "..." {
            break;
        }
        if let Some(rest) = line.strip_prefix(field) {
            if rest.trim_start().starts_with(':') {
                return Some((idx + 2) as u32);
            }
        }
    }
    None
}

/// Returns the 1-based line number where the markdown body starts (after frontmatter).
pub(crate) fn body_start_line(raw: &str) -> u32 {
    let offset = body_start_offset(raw);
    (raw[..offset].matches('\n').count() + 1) as u32
}

/// Finds the 1-based `(line, col)` of a `{> frag_name <}` include in `raw`.
fn find_fragment_include_location(raw: &str, frag_name: &str) -> Option<(u32, u32)> {
    let body_offset = body_start_offset(raw);
    let body = &raw[body_offset..];
    let fm_line_offset = raw[..body_offset].matches('\n').count() as u32;

    let body_source = PanSource::new(body.to_string());
    let mut parser = PanParse::new(&body_source);
    parser.parse();

    for node in &parser.nodes {
        if let Node::Fragment {
            value,
            source_range,
        } = node
        {
            if value.trim() == frag_name {
                let loc = body_source.location_at(source_range.start);
                return Some((loc.line + fm_line_offset, loc.column));
            }
        }
    }
    None
}

/// Constructs a [`Diagnostic`] without location information.
pub fn diag(severity: Severity, skill: &str, rule: &str, message: String) -> Diagnostic {
    Diagnostic {
        rule: rule.to_string(),
        severity,
        skill: skill.to_string(),
        message,
        path: None,
        line: None,
        col: None,
        duplicated_text: None,
        affected_skills: None,
        related: None,
    }
}

/// Constructs a [`Diagnostic`] with optional file/line/column location.
pub fn diag_located(
    severity: Severity,
    skill: &str,
    rule: &str,
    message: String,
    path: Option<String>,
    line: Option<u32>,
    col: Option<u32>,
) -> Diagnostic {
    Diagnostic {
        rule: rule.to_string(),
        severity,
        skill: skill.to_string(),
        message,
        path,
        line,
        col,
        duplicated_text: None,
        affected_skills: None,
        related: None,
    }
}
