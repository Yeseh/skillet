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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;
    use crate::lint::pipeline::SourceInput;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn make_source_file(dir: &Path, name: &str, content: &str) -> pipeline::SourceFile {
        let skill_dir = dir.join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        let input = SourceInput {
            name: name.to_string(),
            source_path,
            skill_dir,
            skill_out_dir: dir.join("skills").join(name),
            content: content.to_string(),
            reference_docs: vec![],
        };
        let files = pipeline::scan_sources(&[input], "cl100k_base");
        let skill_names = vec![name];
        let (mut files, _) = pipeline::extract_refs(files, &skill_names);
        files.remove(0)
    }

    #[allow(dead_code)]
    fn init_workspace(dir: &Path) {
        let cfg = SkilletConfig::default();
        fs::write(dir.join("skillet.toml"), cfg.to_toml().unwrap()).unwrap();
        fs::create_dir_all(dir.join(&cfg.workspace.skills_src_dir)).unwrap();
        fs::create_dir_all(dir.join(&cfg.workspace.skills_out_dir)).unwrap();
        fs::create_dir_all(dir.join(&cfg.workspace.fragments_dir)).unwrap();
    }

    // ── rules::invalid_frontmatter ───────────────────────────────────────────

    #[test]
    fn check_frontmatter_passes_for_valid_skill() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: does things\n---\n\n# body\n",
        );
        let diags = rules::invalid_frontmatter::check(&sf, &SkilletConfig::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn check_frontmatter_errors_on_name_mismatch() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: wrong\ndescription: x\n---\n",
        );
        let diags = rules::invalid_frontmatter::check(&sf, &SkilletConfig::default());
        assert!(diags
            .iter()
            .any(|d| d.rule == "invalid-frontmatter" && d.severity == Severity::Error));
    }

    #[test]
    fn check_frontmatter_errors_on_missing_description() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(tmp.path(), "my-skill", "---\nname: my-skill\n---\n");
        let diags = rules::invalid_frontmatter::check(&sf, &SkilletConfig::default());
        assert!(diags.iter().any(|d| d.rule == "invalid-frontmatter"));
    }

    // ── rules::stale_refs ────────────────────────────────────────────────────

    #[test]
    fn check_refs_errors_on_missing_path_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `ref::missing.sh`\n",
        );
        let ctx = LintContext::default();
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(diags.iter().any(|d| d.rule == "stale-path-ref"));
    }

    #[test]
    fn check_refs_passes_for_existing_path_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `ref::helper.sh`\n",
        );
        let mut ctx = LintContext::default();
        let mut files = HashSet::new();
        files.insert("helper.sh".to_string());
        ctx.skill_files.insert("my-skill".to_string(), files);
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(diags.is_empty());
    }

    #[test]
    fn check_refs_errors_on_missing_skill_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `skill::nonexistent`\n",
        );
        let ctx = LintContext::default();
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(diags.iter().any(|d| d.rule == "stale-skill-ref"));
    }

    #[test]
    fn check_refs_errors_on_undeclared_var_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `var::missing_var`\n",
        );
        let ctx = LintContext::default();
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(diags.iter().any(|d| d.rule == "stale-var-ref"));
    }

    #[test]
    fn check_refs_passes_for_declared_var_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nProject: `var::project_name`\n",
        );
        let ctx = LintContext::default();
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(!diags.iter().any(|d| d.rule == "stale-var-ref"));
    }

    #[test]
    fn check_refs_errors_on_undeclared_env_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nCI: `env::UNKNOWN_ENV`\n",
        );
        let ctx = LintContext::default();
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(diags.iter().any(|d| d.rule == "stale-env-ref"));
    }

    #[test]
    fn check_refs_passes_for_declared_env_ref() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nCI: `env::CI`\n",
        );
        let ctx = LintContext::default();
        let diags = rules::stale_refs::check(&sf, &SkilletConfig::default(), &ctx);
        assert!(!diags.iter().any(|d| d.rule == "stale-env-ref"));
    }

    #[test]
    fn check_unused_fragments_warns_on_unreferenced_fragment() {
        let mut ctx = LintContext::default();
        ctx.fragment_names.push("unused".to_string());
        let diags = rules::unused_fragment::check(&[], &ctx, &SkilletConfig::default());
        assert!(diags
            .iter()
            .any(|d| d.rule == "unused-fragment" && d.message.contains("unused")));
    }

    #[test]
    fn check_unused_fragments_silent_when_fragment_is_used() {
        let tmp = TempDir::new().unwrap();
        let sf = make_source_file(
            tmp.path(),
            "diagnose",
            "---\nname: diagnose\ndescription: x\n---\n\n{{> note }}\n",
        );
        let mut ctx = LintContext::default();
        ctx.fragment_names.push("note".to_string());
        let diags = rules::unused_fragment::check(&[sf], &ctx, &SkilletConfig::default());
        assert!(diags.is_empty());
    }
}
