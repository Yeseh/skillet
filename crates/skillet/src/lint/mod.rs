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

use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::compiler::check::{check_source_file, CheckDiag, CheckKind};
use crate::compiler::compile::{compile, CompileOutput};
use crate::compiler::PanSource;
use crate::config::SkilletConfig;
use crate::freshness;
use crate::lockfile::Lockfile;
use crate::workspace::{Skill, Workspace};

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
    /// Absolute path to the `.pan` source file.
    pub source_path: PathBuf,
    /// Raw source content (empty if unreadable).
    pub raw: String,
    /// Referential-integrity diagnostics from the check stage.
    pub check_diags: Vec<CheckDiag>,
    /// Output of the compile stage.
    pub output: CompileOutput,
}

fn compile_skill(skill: &Skill, ws: &Workspace) -> CompiledSkill {
    let raw = std::fs::read_to_string(&skill.source_path).unwrap_or_default();
    let source = PanSource::new(raw.clone());

    let known_files = ws.get_source_files_for_skill(skill);
    let check_diags = check_source_file(ws, &source, &known_files);
    let output = compile(ws, &source);

    CompiledSkill {
        name: skill.name.clone(),
        source_path: skill.source_path.clone(),
        raw,
        check_diags,
        output,
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
    let targets: Vec<&Skill> = match (&opts.file_path, &opts.skill) {
        (Some(path), _) => {
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                ws.root.join(path)
            };
            ws.skills.values().filter(|s| s.source_path == abs).collect()
        }
        (None, Some(name)) => ws.skills.values().filter(|s| &s.name == name).collect(),
        (None, None) => ws.skills.values().collect(),
    };

    let run_workspace_rules = opts.file_path.is_none() && opts.skill.is_none();
    let target_names: HashSet<&str> = targets.iter().map(|s| s.name.as_str()).collect();

    let compiled: Vec<CompiledSkill> =
        targets.par_iter().map(|s| compile_skill(s, ws)).collect();

    // Per-skill rules and workspace rules run concurrently.
    let (mut diagnostics, (workspace_diags, updated_minhash)) = rayon::join(
        || -> Vec<Diagnostic> {
            let skill_names: Vec<&str> = ws.skills.keys().map(|s| s.as_str()).collect();
            compiled
                .par_iter()
                .flat_map(|cs| lint_skill_rules(cs, config, &skill_names))
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

    // `stale-build` reuses the freshness verifier (the `skillet check` logic).
    diagnostics.extend(stale_build_diags(ws, lockfile, config, &target_names, run_workspace_rules));

    LintOutput {
        diagnostics,
        updated_minhash,
    }
}

fn lint_skill_rules(
    cs: &CompiledSkill,
    config: &SkilletConfig,
    skill_names: &[&str],
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let file_path = cs.source_path.to_string_lossy().to_string();

    diags.extend(rules::invalid_frontmatter::check(&cs.name, &cs.raw));

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
    config: &SkilletConfig,
    target_names: &HashSet<&str>,
    run_workspace_rules: bool,
) -> Vec<Diagnostic> {
    let fragments_dir = ws.root.join(&config.workspace.fragments_dir);
    let report = freshness::verify(ws, lockfile, &fragments_dir);

    report
        .skills
        .iter()
        .filter(|r| !r.fresh)
        .filter(|r| run_workspace_rules || target_names.contains(r.name.as_str()))
        .flat_map(|r| {
            r.reasons.iter().map(move |reason| {
                diag(Severity::Error, &r.name, "stale-build", reason.clone())
            })
        })
        .collect()
}

// ── Shared helpers ────────────────────────────────────────────────────────────

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
    }
}
