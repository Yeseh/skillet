//! CLI orchestration for `skillet lint`.
//!
//! Handles discovery, lockfile I/O, timing, rendering, and severity filtering.
//! Delegates rule execution to the library's lint pipeline and rules.

pub use skillet::lint::{Diagnostic, LintOptions, OutputFormat, Severity};

use anyhow::Result;
use owo_colors::OwoColorize;
use rayon::prelude::*;
use sha2::Digest;
use skillet::config::SkilletConfig;
use skillet::lint::{pipeline, rules, LintContext};
use skillet::lockfile;
use skillet::tokens::count_tokens;
use skillet::workspace::{ResolvedWorkspace, Skill};
use std::path::Path;

/// Runs all enabled lint rules across the workspace (or a single skill/file).
///
/// Returns `Ok(true)` when the workspace is clean (no errors after severity
/// promotion).
pub fn run(
    workspace_path: &Path,
    skill_name: Option<&str>,
    opts: &LintOptions,
    config: &SkilletConfig,
) -> Result<bool> {
    let total_start = std::time::Instant::now();

    let ws = ResolvedWorkspace::resolve(workspace_path, config)?;
    let mut lockfile = lockfile::read(workspace_path)?;

    let scan_targets: Vec<&Skill> = match (&opts.file_path, skill_name) {
        (Some(path), _) => {
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                workspace_path.join(path)
            };
            ws.skills.iter().filter(|s| s.source_path == abs).collect()
        }
        (None, Some(name)) => ws.skills.iter().filter(|s| s.name == name).collect(),
        (None, None) => ws.skills.iter().collect(),
    };

    // ── Build LintContext from ResolvedWorkspace ─────────────────────────────
    let ctx = build_lint_context(&ws, config)?;

    // ── Build SourceInputs (pre-read all files) ──────────────────────────────
    let inputs: Vec<pipeline::SourceInput> =
        scan_targets.iter().map(|s| build_source_input(s)).collect();

    // ── Phase 1: Parallel source scan ────────────────────────────────────────
    let p1_start = std::time::Instant::now();
    let source_files = pipeline::scan_sources(&inputs, &config.build.tokenizer);
    let p1_elapsed = p1_start.elapsed();

    // ── Phase 2: Parallel ref extraction ─────────────────────────────────────
    let p2_start = std::time::Instant::now();
    let skill_names: Vec<&str> = ws.skills.iter().map(|s| s.name.as_str()).collect();
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
                .flat_map(|sf| lint_skill(sf, config, &lockfile, &ctx))
                .collect()
        },
        || -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
            if run_workspace_rules {
                lint_workspace(config, &source_files, &lockfile, &ctx)
            } else {
                (vec![], vec![])
            }
        },
    );

    let p3_elapsed = p3_start.elapsed();

    let mut diagnostics: Vec<Diagnostic> = branch_a;
    diagnostics.extend(branch_b);

    // Write back updated MinHash signatures to lockfile.
    if !dup_updated_sigs.is_empty() {
        let mut lockfile_modified = false;
        for (skill_nm, sig) in dup_updated_sigs {
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

// ── LintContext construction ──────────────────────────────────────────────────

fn build_lint_context(ws: &ResolvedWorkspace, config: &SkilletConfig) -> Result<LintContext> {
    let mut ctx = LintContext::default();

    // Skill files and known dirs from the resolved workspace.
    for skill in &ws.skills {
        let files = ws.skill_files(skill);
        ctx.skill_files.insert(skill.name.clone(), files);
        ctx.known_skill_dirs.insert(skill.name.clone());
    }

    // Known commands from workspace-level scan.
    ctx.known_commands = ws.known_commands.clone();

    // Compiled SKILL.md hashes and texts.
    for skill in &ws.skills {
        let path = skill.skill_out_dir.join("SKILL.md");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let hash = format!(
                "sha256:{}",
                hex::encode(sha2::Sha256::digest(text.as_bytes()))
            );
            ctx.compiled_hashes.insert(skill.name.clone(), hash);
            ctx.compiled_texts.insert(skill.name.clone(), text);
        }
    }

    // Activation tokens from compiled texts.
    for (name, text) in &ctx.compiled_texts {
        let tokens = count_tokens(text, &config.build.tokenizer);
        ctx.activation_tokens.insert(name.clone(), tokens);
    }

    // Fragment data from the resolved workspace.
    ctx.fragment_names = ws
        .fragment_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    ctx.fragment_hashes = ws.fragment_hashes.clone();
    ctx.fragment_tokens = ws.fragment_tokens.clone();

    Ok(ctx)
}

/// Pre-reads source file and reference docs into a `SourceInput`.
fn build_source_input(skill: &Skill) -> pipeline::SourceInput {
    let content = std::fs::read_to_string(&skill.source_path).unwrap_or_default();

    let mut reference_docs = Vec::new();
    let ref_dir = skill.skill_dir.join("reference");
    if ref_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&ref_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        reference_docs.push((path, text));
                    }
                }
            }
        }
    }

    pipeline::SourceInput {
        name: skill.name.clone(),
        source_path: skill.source_path.clone(),
        skill_dir: skill.skill_dir.clone(),
        skill_out_dir: skill.skill_out_dir.clone(),
        content,
        reference_docs,
    }
}

// ── Per-skill lint pass ───────────────────────────────────────────────────────

fn lint_skill(
    source: &pipeline::SourceFile,
    config: &SkilletConfig,
    lockfile: &lockfile::Lockfile,
    ctx: &LintContext,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    if !source.parse_errors.is_empty() && source.raw.is_empty() {
        diags.extend(rules::invalid_frontmatter::check(source, config));
        return diags;
    }

    diags.extend(rules::invalid_frontmatter::check(source, config));
    diags.extend(rules::stale_refs::check(source, config, ctx));
    diags.extend(rules::markdown_links::check(source, config, ctx));
    diags.extend(rules::untyped_backtick::check(source));
    diags.extend(rules::stale_build::check(source, lockfile, ctx));
    diags.extend(rules::oversized::check_skill(source, config, lockfile, ctx));
    diags.extend(rules::oversized::check_description(source, config));

    diags
}

// ── Workspace-level lint pass ─────────────────────────────────────────────────

fn lint_workspace(
    config: &SkilletConfig,
    source_files: &[pipeline::SourceFile],
    lockfile: &lockfile::Lockfile,
    ctx: &LintContext,
) -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
    let mut diags = Vec::new();
    diags.extend(rules::unused_fragment::check(source_files, ctx, config));
    diags.extend(rules::oversized::check_fragments(config, ctx));
    let (dup_diags, updated_sigs) = rules::duplication::check(lockfile, ctx);
    diags.extend(dup_diags);
    (diags, updated_sigs)
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
