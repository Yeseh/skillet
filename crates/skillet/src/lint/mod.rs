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

use crate::config::SkilletConfig;
use crate::workspace::{self, SkillSource};
use anyhow::Result;
use owo_colors::OwoColorize;
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub mod pipeline;
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

// ── Entry point ───────────────────────────────────────────────────────────────

/// Runs all enabled lint rules across the workspace (or a single skill/file).
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
    let total_start = std::time::Instant::now();

    let config = crate::config::load(workspace)?;
    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);
    let mut lockfile = crate::lockfile::read(workspace)?;

    let all_sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;

    // Determine which skills to scan.  In --file mode we find the owning skill
    // by matching source_path; the full all_sources list still provides skill
    // names for cross-ref validation.
    let scan_targets: Vec<&SkillSource> = match (&opts.file_path, skill_name) {
        (Some(path), _) => {
            // Canonicalise so relative and absolute paths both match.
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                workspace.join(path)
            };
            all_sources
                .iter()
                .filter(|s| s.source_path == abs)
                .collect()
        }
        (None, Some(name)) => all_sources.iter().filter(|s| s.name == name).collect(),
        (None, None) => all_sources.iter().collect(),
    };

    // ── Phase 1: Parallel source scan ────────────────────────────────────────
    let p1_start = std::time::Instant::now();
    let source_files = pipeline::scan_sources(
        &scan_targets
            .iter()
            .map(|s| (*s).clone())
            .collect::<Vec<_>>(),
        &config.build.tokenizer,
    );
    let p1_elapsed = p1_start.elapsed();

    // ── Phase 2: Parallel ref extraction ─────────────────────────────────────
    let p2_start = std::time::Instant::now();
    let skill_names: Vec<&str> = all_sources.iter().map(|s| s.name.as_str()).collect();
    let (source_files, _all_refs) = pipeline::extract_refs(source_files, &skill_names);
    let p2_elapsed = p2_start.elapsed();

    let files_scanned = source_files.len();

    // ── Phase 3: rayon::join(branch A, branch B) ─────────────────────────────
    let p3_start = std::time::Instant::now();

    let run_workspace_rules = opts.file_path.is_none() && skill_name.is_none();

    let (branch_a, (branch_b, dup_updated_sigs)) = rayon::join(
        || -> Vec<Diagnostic> {
            source_files
                .par_iter()
                .filter(|sf| matches!(sf.file_type, pipeline::SourceFileType::Skill))
                .flat_map(|sf| {
                    lint_skill(
                        sf,
                        &config,
                        &all_sources,
                        &fragments_dir,
                        &skills_src_dir,
                        &lockfile,
                    )
                })
                .collect()
        },
        || -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
            if run_workspace_rules {
                lint_workspace(
                    &config,
                    &all_sources,
                    &source_files,
                    &fragments_dir,
                    &lockfile,
                )
            } else {
                (vec![], vec![])
            }
        },
    );

    let p3_elapsed = p3_start.elapsed();

    let mut diagnostics: Vec<Diagnostic> = branch_a;
    diagnostics.extend(branch_b);

    // Write back updated MinHash signatures to lockfile if any were computed.
    if !dup_updated_sigs.is_empty() {
        let mut lockfile_modified = false;
        for (skill_nm, sig) in dup_updated_sigs {
            if let Some(entry) = lockfile.skills.get_mut(&skill_nm) {
                entry.minhash = sig;
                lockfile_modified = true;
            }
        }
        if lockfile_modified {
            let _ = crate::lockfile::write(workspace, &lockfile);
        }
    }

    // Drop rules disabled in skillet.toml.
    diagnostics.retain(|d| !config.lint.disable.contains(&d.rule));

    // Strict mode: promote warnings to errors.
    if opts.strict {
        for d in &mut diagnostics {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    // Drop info diagnostics unless --pedantic.
    if !opts.pedantic {
        diagnostics.retain(|d| d.severity != Severity::Info);
    }

    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);
    let total_elapsed = total_start.elapsed();

    match opts.format {
        OutputFormat::Text => print_text(
            &diagnostics,
            files_scanned,
            total_elapsed.as_millis(),
            opts.verbose.then_some((p1_elapsed, p2_elapsed, p3_elapsed)),
        ),
        OutputFormat::Json => print_json(&diagnostics)?,
        OutputFormat::Silent => {}
    }

    Ok(!has_errors)
}

// ── Per-skill lint pass ───────────────────────────────────────────────────────

fn lint_skill(
    source: &pipeline::SourceFile,
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    fragments_dir: &Path,
    skills_src_dir: &Path,
    lockfile: &crate::lockfile::Lockfile,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    // If Phase 1 had a read error, surface it and stop.
    if !source.parse_errors.is_empty() && source.raw.is_empty() {
        diags.extend(rules::invalid_frontmatter::check(source, config));
        return diags;
    }

    diags.extend(rules::invalid_frontmatter::check(source, config));
    diags.extend(rules::stale_refs::check(
        source,
        config,
        all_sources,
        skills_src_dir,
    ));
    diags.extend(rules::markdown_links::check(source, config));
    diags.extend(rules::untyped_backtick::check(source));
    diags.extend(rules::stale_build::check(source, fragments_dir, lockfile));
    diags.extend(rules::oversized::check_skill(source, config, lockfile));
    diags.extend(rules::oversized::check_description(source, config));

    diags
}

// ── Workspace-level lint pass ─────────────────────────────────────────────────

fn lint_workspace(
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    source_files: &[pipeline::SourceFile],
    fragments_dir: &Path,
    lockfile: &crate::lockfile::Lockfile,
) -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
    let mut diags = Vec::new();
    diags.extend(rules::unused_fragment::check(
        source_files,
        fragments_dir,
        config,
    ));
    diags.extend(rules::oversized::check_fragments(config, fragments_dir));
    let (dup_diags, updated_sigs) = rules::duplication::check(all_sources, lockfile);
    diags.extend(dup_diags);
    (diags, updated_sigs)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

pub(crate) fn diag(severity: Severity, skill: &str, rule: &str, message: String) -> Diagnostic {
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

pub(crate) fn diag_located(
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

// ── Output ────────────────────────────────────────────────────────────────────

fn print_text(
    diagnostics: &[Diagnostic],
    files_scanned: usize,
    elapsed_ms: u128,
    phase_timings: Option<(
        std::time::Duration,
        std::time::Duration,
        std::time::Duration,
    )>,
) {
    if diagnostics.is_empty() {
        println!("{}", "no issues found".green());
    } else {
        for d in diagnostics {
            let tag = match d.severity {
                Severity::Error => "error".red().bold().to_string(),
                Severity::Warning => "warning".yellow().bold().to_string(),
                Severity::Info => "info".cyan().bold().to_string(),
            };
            let location = match (&d.path, d.line, d.col) {
                (Some(p), Some(l), Some(c)) => format!(" ({}:{}:{})", p, l, c),
                (Some(p), Some(l), None) => format!(" ({}:{})", p, l),
                (Some(p), None, _) => format!(" ({})", p),
                _ => String::new(),
            };
            println!(
                "[{tag}] {} ({}) {}{}",
                d.skill, d.rule, d.message, location
            );
        }
        let errors = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        let infos = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Info)
            .count();
        if infos > 0 {
            println!(
                "\n{} error(s), {} warning(s), {} info(s)",
                errors, warnings, infos
            );
        } else {
            println!("\n{} error(s), {} warning(s)", errors, warnings);
        }
    }
    println!(
        "{}",
        format!(
            "scanned {} file{} in {}ms",
            files_scanned,
            if files_scanned == 1 { "" } else { "s" },
            elapsed_ms
        )
        .dimmed()
    );
    if let Some((p1, p2, p3)) = phase_timings {
        println!(
            "{}",
            format!(
                "  phase 1 (scan): {}ms  phase 2 (refs): {}ms  phase 3 (rules): {}ms",
                p1.as_millis(),
                p2.as_millis(),
                p3.as_millis(),
            )
            .dimmed()
        );
    }
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

    fn make_source_file(dir: &Path, name: &str, content: &str) -> pipeline::SourceFile {
        let skill_dir = dir.join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        fs::write(&source_path, content).unwrap();
        let skill_out_dir = dir.join("skills").join(name);
        let src = SkillSource {
            name: name.to_string(),
            source_path,
            skill_dir,
            skill_out_dir,
        };
        let files = pipeline::scan_sources(&[src], "cl100k_base");
        let skill_names = vec![name];
        let (mut files, _) = pipeline::extract_refs(files, &skill_names);
        files.remove(0)
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
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
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
        fs::write(sf.skill_dir.join("helper.sh"), "").unwrap();
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
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
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
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
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
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
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
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
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
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
        let diags = rules::stale_refs::check(
            &sf,
            &SkilletConfig::default(),
            &[],
            &tmp.path().join("src/skills"),
        );
        assert!(!diags.iter().any(|d| d.rule == "stale-env-ref"));
    }

    #[test]
    fn check_unused_fragments_warns_on_unreferenced_fragment() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("unused.fragment.pan"), "# unused\n").unwrap();
        let diags = rules::unused_fragment::check(&[], tmp.path(), &SkilletConfig::default());
        assert!(diags
            .iter()
            .any(|d| d.rule == "unused-fragment" && d.message.contains("unused")));
    }

    #[test]
    fn check_unused_fragments_silent_when_fragment_is_used() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("note.fragment.pan"), "# note\n").unwrap();
        let sf = make_source_file(
            tmp.path(),
            "diagnose",
            "---\nname: diagnose\ndescription: x\n---\n\n{{> note }}\n",
        );
        let diags = rules::unused_fragment::check(&[sf], tmp.path(), &SkilletConfig::default());
        assert!(diags.is_empty());
    }

    // ── run ──────────────────────────────────────────────────────────────────

    #[test]
    fn run_returns_true_for_clean_workspace() {
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

        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();
        assert!(clean);
    }

    #[test]
    fn run_returns_false_when_errors_present() {
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/bad");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("bad.pan"),
            "---\nname: wrong-name\ndescription: x\n---\n\n# Bad\n",
        )
        .unwrap();

        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();
        assert!(!clean);
    }

    #[test]
    fn run_strict_promotes_warnings_to_errors() {
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

        let clean_normal = run(
            tmp.path(),
            None,
            &LintOptions {
                strict: false,
                ..Default::default()
            },
        )
        .unwrap();
        let clean_strict = run(
            tmp.path(),
            None,
            &LintOptions {
                strict: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(clean_normal);
        assert!(clean_strict);
    }

    #[test]
    fn run_disabled_rule_suppresses_diagnostic() {
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

        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();
        assert!(clean);
    }

    #[test]
    fn run_stale_build_fires_when_skill_md_missing() {
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: x\n---\n\n# body\n",
        )
        .unwrap();

        let clean = run(tmp.path(), None, &LintOptions::default()).unwrap();
        assert!(!clean);
    }

    #[test]
    fn run_file_mode_lints_single_file() {
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        let pan_path = skill_src_dir.join("my-skill.pan");
        fs::write(
            &pan_path,
            "---\nname: wrong-name\ndescription: x\n---\n\n# body\n",
        )
        .unwrap();

        let opts = LintOptions {
            file_path: Some(pan_path),
            format: OutputFormat::Silent,
            ..Default::default()
        };
        let clean = run(tmp.path(), None, &opts).unwrap();
        assert!(!clean, "invalid-frontmatter should fire in --file mode");
    }
}
