//! Lint engine for skill quality validation.
//!
//! The engine runs a three-phase pipeline:
//!
//! 1. **Phase 1** — Parallel source scan: reads every file, hashes it, counts
//!    tokens, and parses frontmatter.
//! 2. **Phase 2** — Parallel ref extraction: extracts all typed refs, markdown
//!    links, and untyped backticks from the in-memory file content.
//! 3. **Phase 3** — Parallel rule execution: runs per-skill rules (branch A)
//!    and workspace rules (branch B) concurrently via `rayon::join`.
//!
//! Individual rule implementations live in the `rules` submodule — one file
//! per rule (or closely-related rule group).

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub mod pipeline;
pub mod rules;

// ── LintContext ───────────────────────────────────────────────────────────────

/// Pre-loaded workspace state for lint rule execution.
/// Built by the CLI; consumed by pure rule functions.
#[derive(Debug, Default)]
pub struct LintContext {
    /// Files known to exist relative to each skill dir.
    /// Key: skill name, Value: set of relative paths.
    pub skill_files: HashMap<String, HashSet<String>>,

    /// Commands confirmed present on PATH.
    pub known_commands: HashSet<String>,

    /// Skill directory names that exist in the workspace.
    pub known_skill_dirs: HashSet<String>,

    /// SHA-256 hash of each compiled SKILL.md (key: skill name).
    pub compiled_hashes: HashMap<String, String>,

    /// Full text of each compiled SKILL.md (key: skill name).
    /// Needed by duplication detection.
    pub compiled_texts: HashMap<String, String>,

    /// SHA-256 hash of each fragment file (key: fragment name).
    pub fragment_hashes: HashMap<String, String>,

    /// Token count per fragment (key: fragment name).
    pub fragment_tokens: HashMap<String, u32>,

    /// All fragment names present in the fragments directory.
    pub fragment_names: Vec<String>,

    /// Activation token count per skill from lockfile (key: skill name).
    /// Used by oversized rule when lockfile data is available.
    pub activation_tokens: HashMap<String, u32>,
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
            file_path: None,
            verbose: false,
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

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
