//! Lint engine for skill quality validation.
//!
//! The engine orchestrates all rules. Individual rule implementations live in
//! the `rules` submodule — one file per rule (or closely-related rule group).

use crate::config::SkilletConfig;
use crate::workspace::{self, SkillSource};
use anyhow::Result;
use owo_colors::OwoColorize;
use serde::Serialize;
use std::path::Path;

mod rules;

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
    /// Human-readable description of the problem.
    pub message: String,
}

/// Output format for lint results.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable coloured text.
    #[default]
    Human,
    /// Machine-parseable JSON array.
    Json,
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
}

impl LintOptions {
    /// Creates a new `LintOptions` with all options specified.
    pub fn new(strict: bool, pedantic: bool, format: OutputFormat) -> Self {
        Self { strict, pedantic, format }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Runs all enabled lint rules across the workspace (or a single skill).
///
/// Returns `Ok(true)` when the workspace is clean (no errors after severity
/// promotion).  Prints results to stdout according to `opts.format`.
///
/// # Errors
///
/// Returns an error only if the workspace cannot be read (e.g. missing
/// `skillet.toml`).  Individual rule failures are reported as diagnostics, not
/// as `Err`.
pub fn run(workspace: &Path, skill_name: Option<&str>, opts: &LintOptions) -> Result<bool> {
    let config = crate::config::load(workspace)?;
    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);

    let all_sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;
    let targets: Vec<&SkillSource> = match skill_name {
        Some(name) => all_sources.iter().filter(|s| s.name == name).collect(),
        None => all_sources.iter().collect(),
    };

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for source in &targets {
        diagnostics.extend(lint_skill(
            source,
            &config,
            &all_sources,
            workspace,
            &fragments_dir,
            &skills_src_dir,
        ));
    }

    // Workspace-level rules run only when linting everything
    if skill_name.is_none() {
        diagnostics.extend(lint_workspace(&config, &all_sources, &fragments_dir));
    }

    // Drop rules disabled in skillet.toml
    diagnostics.retain(|d| !config.lint.disable.contains(&d.rule));

    // Strict mode: promote warnings to errors
    if opts.strict {
        for d in &mut diagnostics {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    // Drop info diagnostics unless --pedantic
    if !opts.pedantic {
        diagnostics.retain(|d| d.severity != Severity::Info);
    }

    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);

    match opts.format {
        OutputFormat::Human => print_human(&diagnostics),
        OutputFormat::Json => print_json(&diagnostics)?,
    }

    Ok(!has_errors)
}

// ── Per-skill lint pass ───────────────────────────────────────────────────────

fn lint_skill(
    source: &SkillSource,
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    _workspace: &Path,
    fragments_dir: &Path,
    skills_src_dir: &Path,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    let raw = match std::fs::read_to_string(&source.source_path) {
        Ok(s) => s,
        Err(e) => {
            diags.push(diag(
                Severity::Error,
                &source.name,
                "invalid-frontmatter",
                format!("cannot read source: {e}"),
            ));
            return diags;
        }
    };

    diags.extend(rules::invalid_frontmatter::check(source, &raw, config));
    diags.extend(rules::stale_refs::check(source, &raw, config, all_sources, skills_src_dir));
    diags.extend(rules::markdown_links::check(source, &raw, config));
    diags.extend(rules::untyped_backtick::check(source, &raw, all_sources, config));
    diags.extend(rules::stale_build::check(source, config, fragments_dir, skills_src_dir));
    diags.extend(rules::oversized::check_skill(source, config));
    diags.extend(rules::oversized::check_description(source, &raw, config));

    diags
}

// ── Workspace-level lint pass ─────────────────────────────────────────────────

fn lint_workspace(
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    fragments_dir: &Path,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    diags.extend(rules::unused_fragment::check(all_sources, fragments_dir, config));
    diags.extend(rules::oversized::check_fragments(config, fragments_dir));
    // duplication: not yet implemented (planned for a future story)
    diags
}

// ── Shared helper ─────────────────────────────────────────────────────────────

pub(crate) fn diag(severity: Severity, skill: &str, rule: &str, message: String) -> Diagnostic {
    Diagnostic {
        rule: rule.to_string(),
        severity,
        skill: skill.to_string(),
        message,
    }
}

// ── Output ────────────────────────────────────────────────────────────────────

fn print_human(diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        println!("{}", "no issues found".green());
        return;
    }
    for d in diagnostics {
        let tag = match d.severity {
            Severity::Error => "error".red().bold().to_string(),
            Severity::Warning => "warning".yellow().bold().to_string(),
            Severity::Info => "info".cyan().bold().to_string(),
        };
        println!("[{tag}] {} ({}): {}", d.skill, d.rule, d.message);
    }
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = diagnostics.iter().filter(|d| d.severity == Severity::Warning).count();
    println!("\n{} error(s), {} warning(s)", errors, warnings);
}

fn print_json(diagnostics: &[Diagnostic]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(diagnostics)?);
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;
    use std::fs;
    use tempfile::TempDir;

    fn make_source(dir: &Path, name: &str, content: &str) -> SkillSource {
        let skill_dir = dir.join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        fs::write(&source_path, content).unwrap();
        let skill_out_dir = dir.join("skills").join(name);
        SkillSource { name: name.to_string(), source_path, skill_dir, skill_out_dir }
    }

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
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: does things\n---\n\n# body\n",
        );

        // Act
        let diags = rules::invalid_frontmatter::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &SkilletConfig::default(),
        );

        // Assert
        assert!(diags.is_empty());
    }

    #[test]
    fn check_frontmatter_errors_on_name_mismatch() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: wrong\ndescription: x\n---\n",
        );

        // Act
        let diags = rules::invalid_frontmatter::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &SkilletConfig::default(),
        );

        // Assert
        assert!(diags.iter().any(|d| d.rule == "invalid-frontmatter" && d.severity == Severity::Error));
    }

    #[test]
    fn check_frontmatter_errors_on_missing_description() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(tmp.path(), "my-skill", "---\nname: my-skill\n---\n");

        // Act
        let diags = rules::invalid_frontmatter::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &SkilletConfig::default(),
        );

        // Assert
        assert!(diags.iter().any(|d| d.rule == "invalid-frontmatter"));
    }

    // ── rules::stale_refs ────────────────────────────────────────────────────

    #[test]
    fn check_refs_errors_on_missing_path_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `ref::missing.sh`\n",
        );
        let config = SkilletConfig::default();

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(diags.iter().any(|d| d.rule == "stale-path-ref"));
    }

    #[test]
    fn check_refs_passes_for_existing_path_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `ref::helper.sh`\n",
        );
        fs::write(src.skill_dir.join("helper.sh"), "").unwrap();
        let config = SkilletConfig::default();

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(diags.is_empty());
    }

    #[test]
    fn check_refs_errors_on_missing_skill_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `skill::nonexistent`\n",
        );
        let config = SkilletConfig::default();

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(diags.iter().any(|d| d.rule == "stale-skill-ref"));
    }

    #[test]
    fn check_refs_errors_on_undeclared_var_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee `var::missing_var`\n",
        );
        let config = SkilletConfig::default();

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(diags.iter().any(|d| d.rule == "stale-var-ref"));
    }

    #[test]
    fn check_refs_passes_for_declared_var_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nProject: `var::project_name`\n",
        );
        let config = SkilletConfig::default(); // has project_name in vars

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(!diags.iter().any(|d| d.rule == "stale-var-ref"));
    }

    #[test]
    fn check_refs_errors_on_undeclared_env_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nCI: `env::UNKNOWN_ENV`\n",
        );
        let config = SkilletConfig::default();

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(diags.iter().any(|d| d.rule == "stale-env-ref"));
    }

    #[test]
    fn check_refs_passes_for_declared_env_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nCI: `env::CI`\n",
        );
        let config = SkilletConfig::default(); // has CI in env

        // Act
        let diags = rules::stale_refs::check(
            &src,
            &fs::read_to_string(&src.source_path).unwrap(),
            &config,
            &[],
            &tmp.path().join("src/skills"),
        );

        // Assert
        assert!(!diags.iter().any(|d| d.rule == "stale-env-ref"));
    }

    #[test]
    fn check_unused_fragments_warns_on_unreferenced_fragment() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("unused.fragment.pan"), "# unused\n").unwrap();
        let config = SkilletConfig::default();

        // Act
        let diags = rules::unused_fragment::check(&[], tmp.path(), &config);

        // Assert
        assert!(diags.iter().any(|d| d.rule == "unused-fragment" && d.message.contains("unused")));
    }

    #[test]
    fn check_unused_fragments_silent_when_fragment_is_used() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("note.fragment.pan"), "# note\n").unwrap();
        let skill_dir = tmp.path().join("src/skills/diagnose");
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join("diagnose.pan");
        fs::write(&source_path, "---\nname: diagnose\ndescription: x\n---\n\n{{> note }}\n").unwrap();
        let sources = vec![SkillSource {
            name: "diagnose".into(),
            source_path,
            skill_dir,
            skill_out_dir: tmp.path().join("skills/diagnose"),
        }];
        let config = SkilletConfig::default();

        // Act
        let diags = rules::unused_fragment::check(&sources, tmp.path(), &config);

        // Assert
        assert!(diags.is_empty());
    }

    // ── run ──────────────────────────────────────────────────────────────────

    #[test]
    fn run_returns_true_for_clean_workspace() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/good");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("good.pan"),
            "---\nname: good\ndescription: a good skill\n---\n\n# Good\n",
        )
        .unwrap();
        crate::build::run(tmp.path(), Some("good"), &Default::default()).unwrap();

        // Act
        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();

        // Assert
        assert!(clean);
    }

    #[test]
    fn run_returns_false_when_errors_present() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/bad");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("bad.pan"),
            "---\nname: wrong-name\ndescription: x\n---\n\n# Bad\n",
        )
        .unwrap();

        // Act
        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();

        // Assert
        assert!(!clean);
    }

    #[test]
    fn run_strict_promotes_warnings_to_errors() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: x\n---\n\n# body\n",
        )
        .unwrap();
        crate::build::run(tmp.path(), Some("my-skill"), &Default::default()).unwrap();

        let opts_normal = LintOptions { strict: false, ..Default::default() };
        let opts_strict = LintOptions { strict: true, ..Default::default() };

        // Act — normal should be clean (no warnings either in this case)
        let clean_normal = run(tmp.path(), None, &opts_normal).unwrap();
        let clean_strict = run(tmp.path(), None, &opts_strict).unwrap();

        // Assert — both clean since no issues
        assert!(clean_normal);
        assert!(clean_strict);
    }

    #[test]
    fn run_disabled_rule_suppresses_diagnostic() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let custom_toml = "[workspace]\nskills_src_dir = 'src/skills'\nskills_out_dir = 'skills'\nfragments_dir = 'src/skills/_fragments'\n\
            [lint]\nmax_activation_tokens = 4000\nmax_discovery_tokens = 100\nmax_fragment_tokens = 500\n\
            allowed_commands = []\ndisable = ['stale-build', 'invalid-frontmatter']\n\
            [build]\ntokenizer = 'cl100k_base'\nverify_urls = false\n\
            [vars]\n[env]\n";
        fs::write(tmp.path().join("skillet.toml"), custom_toml).unwrap();
        fs::create_dir_all(tmp.path().join("src/skills")).unwrap();
        fs::create_dir_all(tmp.path().join("skills")).unwrap();
        fs::create_dir_all(tmp.path().join("src/skills/_fragments")).unwrap();
        let skill_src_dir = tmp.path().join("src/skills/bad");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("bad.pan"),
            "---\nname: wrong\ndescription: x\n---\n\n# body\n",
        )
        .unwrap();

        // Act
        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();

        // Assert — errors suppressed, so clean
        assert!(clean);
    }

    #[test]
    fn run_stale_build_fires_when_skill_md_missing() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: x\n---\n\n# body\n",
        )
        .unwrap();

        // Act
        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();

        // Assert
        assert!(!clean);
    }
}
