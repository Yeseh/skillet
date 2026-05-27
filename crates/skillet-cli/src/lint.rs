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
use skillet::workspace::{self, SkillSource};
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

/// Runs all enabled lint rules across the workspace (or a single skill/file).
///
/// Returns `Ok(true)` when the workspace is clean (no errors after severity
/// promotion).
pub fn run(
    workspace: &Path,
    skill_name: Option<&str>,
    opts: &LintOptions,
    config: &SkilletConfig,
) -> Result<bool> {
    let total_start = std::time::Instant::now();

    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);
    let mut lockfile = lockfile::read(workspace)?;

    let all_sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;

    let scan_targets: Vec<&SkillSource> = match (&opts.file_path, skill_name) {
        (Some(path), _) => {
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

    // ── Build LintContext (all I/O happens here) ─────────────────────────────
    let ctx = build_lint_context(&all_sources, &fragments_dir, config)?;

    // ── Build SourceInputs (pre-read all files) ──────────────────────────────
    let inputs: Vec<pipeline::SourceInput> =
        scan_targets.iter().map(|s| build_source_input(s)).collect();

    // ── Phase 1: Parallel source scan ────────────────────────────────────────
    let p1_start = std::time::Instant::now();
    let source_files = pipeline::scan_sources(&inputs, &config.build.tokenizer);
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
            let _ = lockfile::write(workspace, &lockfile);
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

fn build_lint_context(
    all_sources: &[SkillSource],
    fragments_dir: &Path,
    config: &SkilletConfig,
) -> Result<LintContext> {
    let mut ctx = LintContext::default();

    // Skill files: walk each skill dir to find relative paths.
    for src in all_sources {
        let mut files = HashSet::new();
        if src.skill_dir.exists() {
            for entry in WalkDir::new(&src.skill_dir)
                .min_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.path().is_file() {
                    if let Ok(rel) = entry.path().strip_prefix(&src.skill_dir) {
                        files.insert(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        ctx.skill_files.insert(src.name.clone(), files);
        ctx.known_skill_dirs.insert(src.name.clone());
    }

    // Known commands: collect all cmd:: refs we need to check, then probe PATH.
    // We defer this — the rule will check ctx.known_commands, so we pre-populate
    // with commands that are actually on PATH. We scan PATH for common commands
    // referenced in skill files. For efficiency, we just check is_on_path for
    // any command the rules encounter. Instead, pre-populate known_commands by
    // scanning PATH directories (but that's expensive). The simpler approach:
    // leave it to the caller or just check on-demand. Since the plan says the
    // CLI populates this, let's collect all unique cmd refs and probe them.
    // But we don't have refs yet at this point... We'll collect commands from
    // the source files after Phase 2. Actually, the remediation plan says
    // `ctx.known_commands.contains(cmd)` replaces `workspace::is_on_path(cmd)`.
    // The simplest approach: eagerly populate with all commands on PATH.
    // But that's too expensive. Instead, let's lazily fill it by checking
    // is_on_path for each command we discover in sources. We can't do that
    // because LintContext is built before Phase 2.
    //
    // Solution: pre-read all source files, extract cmd:: refs, then check PATH.
    // But that duplicates Phase 2. Better: just check all possible commands
    // from all source files' raw content via a quick regex scan.
    let cmd_re = regex::Regex::new(r"`cmd::([^`]+)`").unwrap();
    for src in all_sources {
        if let Ok(content) = std::fs::read_to_string(&src.source_path) {
            for caps in cmd_re.captures_iter(&content) {
                let full_cmd = caps[1].trim();
                let cmd = full_cmd.split_whitespace().next().unwrap_or(full_cmd);
                if !ctx.known_commands.contains(cmd) && workspace::is_on_path(cmd) {
                    ctx.known_commands.insert(cmd.to_string());
                }
            }
        }
    }

    // Compiled SKILL.md hashes and texts.
    for src in all_sources {
        let path = src.skill_out_dir.join("SKILL.md");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let hash = format!(
                "sha256:{}",
                hex::encode(sha2::Sha256::digest(text.as_bytes()))
            );
            ctx.compiled_hashes.insert(src.name.clone(), hash);
            ctx.compiled_texts.insert(src.name.clone(), text);
        }
    }

    // Activation tokens from compiled texts (fallback when lockfile doesn't
    // have them).
    for (name, text) in &ctx.compiled_texts {
        let tokens = count_tokens(text, &config.build.tokenizer);
        ctx.activation_tokens.insert(name.clone(), tokens);
    }

    // Fragment data.
    if fragments_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(fragments_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let fname = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let frag_name = match fname.strip_suffix(".fragment.pan") {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                ctx.fragment_names.push(frag_name.clone());

                if let Ok(content) = std::fs::read_to_string(&path) {
                    let hash = format!(
                        "sha256:{}",
                        hex::encode(sha2::Sha256::digest(content.as_bytes()))
                    );
                    let tokens = count_tokens(&content, &config.build.tokenizer);
                    ctx.fragment_hashes.insert(frag_name.clone(), hash);
                    ctx.fragment_tokens.insert(frag_name, tokens);
                }
            }
        }
    }

    Ok(ctx)
}

/// Pre-reads source file and reference docs into a `SourceInput`.
fn build_source_input(src: &SkillSource) -> pipeline::SourceInput {
    let content = std::fs::read_to_string(&src.source_path).unwrap_or_default();

    let mut reference_docs = Vec::new();
    let ref_dir = src.skill_dir.join("reference");
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
        name: src.name.clone(),
        source_path: src.source_path.clone(),
        skill_dir: src.skill_dir.clone(),
        skill_out_dir: src.skill_out_dir.clone(),
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
