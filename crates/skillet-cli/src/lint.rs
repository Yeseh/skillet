//! CLI orchestration for `skillet lint`.
//!
//! Resolves the workspace (as build and check do), reads the lockfile, runs the
//! library lint engine, then handles severity promotion, rule filtering,
//! rendering, and lockfile writeback.

pub use skillet::lint::{Diagnostic, LintOptions, OutputFormat, Severity};

use anyhow::Result;
use owo_colors::OwoColorize;
use skillet::config::SkilletConfig;
use skillet::lint;
use skillet::lockfile;
use skillet::workspace::Workspace;
use std::path::Path;

/// Runs all enabled lint rules across the workspace (or a single skill/file/module).
///
/// Returns `Ok(true)` when the workspace is clean (no errors after severity
/// promotion).
pub fn run(
    workspace_path: &Path,
    skill_name: Option<&str>,
    module_name: Option<&str>,
    opts: &LintOptions,
    config: &SkilletConfig,
) -> Result<bool> {
    let total_start = std::time::Instant::now();

    let ws = Workspace::resolve(workspace_path, config)?;
    let mut lockfile = lockfile::read(workspace_path)?;

    // The library entry takes the target selectors via LintOptions; the CLI's
    // `name` and `module` arguments supply the filters.
    let mut lint_opts = LintOptions::new(opts.strict, opts.pedantic, opts.format.clone());
    lint_opts.skill = skill_name.map(str::to_string);
    lint_opts.module = module_name.map(str::to_string);
    lint_opts.file_path = opts.file_path.clone();
    lint_opts.verbose = opts.verbose;

    let output = lint::lint(&ws, &lockfile, config, &lint_opts);
    let mut diagnostics = output.diagnostics;

    // Write back updated MinHash signatures to the lockfile.
    if !output.updated_minhash.is_empty() {
        let mut lockfile_modified = false;
        for (skill_nm, sig) in output.updated_minhash {
            if let Some(entry) = lockfile.skills.get_mut(&skill_nm) {
                entry.minhash = sig;
                lockfile_modified = true;
            }
        }
        if lockfile_modified {
            let _ = lockfile::write(workspace_path, &lockfile);
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
        OutputFormat::Text => print_text(&diagnostics, total_elapsed.as_millis()),
        OutputFormat::Json => print_json(&diagnostics)?,
        OutputFormat::Silent => {}
    }

    Ok(!has_errors)
}

// ── Output ────────────────────────────────────────────────────────────────────

fn print_text(diagnostics: &[Diagnostic], elapsed_ms: u128) {
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
            println!("[{tag}] {} ({}) {}{}", d.skill, d.rule, d.message, location);
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
    println!("{}", format!("lint completed in {}ms", elapsed_ms).dimmed());
}

fn print_json(diagnostics: &[Diagnostic]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(diagnostics)?);
    Ok(())
}
