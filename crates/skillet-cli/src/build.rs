//! Build orchestration: resolves the workspace, compiles skills, writes outputs,
//! and updates `skillet.lock`.
//!
//! All filesystem I/O for the build pipeline lives here. The pure compilation
//! logic is delegated to `skillet::compiler::compile_pan()`.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use owo_colors::OwoColorize;
use serde::Serialize;
use skillet::compiler::{compile_pan, CompileContext, PanSource};
use skillet::config::SkilletConfig;
use skillet::lockfile::{self, ArtefactEntry, FragmentLockEntry, LockMeta, Lockfile};
use skillet::tokens;
use skillet::workspace::{hash_bytes, hash_file, ResolvedWorkspace, Skill};
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

// ── Build options ──────────────────────────────────────────────────────────────

/// Output format for build results.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

/// Options controlling build behaviour.
#[derive(Debug, Default)]
pub struct BuildOptions {
    pub offline: bool,
    pub strict: bool,
    pub format: OutputFormat,
}

impl BuildOptions {
    pub fn new_with_format(offline: bool, strict: bool, format: OutputFormat) -> Self {
        Self {
            offline,
            strict,
            format,
        }
    }
}

// ── Build report ───────────────────────────────────────────────────────────────

/// Structured report produced by a build run.
#[derive(Debug, Serialize)]
pub struct BuildReport {
    /// Names of skills that were built during this run.
    pub skills_built: Vec<String>,
    /// Warnings generated during the build (e.g. missing commands or broken URLs).
    pub warnings: Vec<String>,
    /// Path to the lockfile used during the build.
    pub lockfile_path: String,
}

/// Compiles `.pan` sources to `SKILL.md` files and updates `skillet.lock`.
pub fn run(
    workspace_path: &Path,
    skill_name: Option<&str>,
    opts: &BuildOptions,
    cfg: &SkilletConfig,
) -> Result<()> {
    let ws = ResolvedWorkspace::resolve(workspace_path, cfg)?;

    let targets: Vec<&Skill> = match skill_name {
        Some(name) => {
            let found = ws.skills.iter().find(|s| s.name == name);
            match found {
                Some(s) => vec![s],
                None => bail!("skill '{}' not found in workspace", name),
            }
        }
        None => ws.skills.iter().collect(),
    };

    if targets.is_empty() {
        if opts.format == OutputFormat::Json {
            let report = BuildReport {
                skills_built: vec![],
                warnings: vec![],
                lockfile_path: workspace_path
                    .join("skillet.lock")
                    .to_string_lossy()
                    .to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            let skills_src_dir = workspace_path.join(&cfg.workspace.skills_src_dir);
            eprintln!("no skills found in {}", skills_src_dir.display());
        }
        return Ok(());
    }

    let known_skills: HashSet<String> = ws
        .skill_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let known_agents: HashSet<String> = ws
        .agent_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let mut lockfile = lockfile::read(workspace_path)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: cfg.build.tokenizer.clone(),
    });

    let mut skills_built: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for skill in &targets {
        compile_one_skill(
            skill,
            cfg,
            workspace_path,
            &ws,
            &known_skills,
            &known_agents,
            &mut lockfile,
        )?;
        if opts.format != OutputFormat::Json {
            println!("built {}", skill.name);
        }
        skills_built.push(skill.name.clone());
    }

    rebuild_fragment_entries(&mut lockfile, &ws)?;

    let lock_path = workspace_path.join("skillet.lock");
    lockfile::write(workspace_path, &lockfile)?;

    if cfg.build.verify_urls && !opts.offline {
        verify_urls_from_lockfile(
            &lockfile,
            opts.strict,
            &mut warnings,
            opts.format != OutputFormat::Json,
        )?;
    }

    if opts.format == OutputFormat::Json {
        let report = BuildReport {
            skills_built,
            warnings,
            lockfile_path: lock_path.to_string_lossy().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn compile_one_skill(
    skill: &Skill,
    cfg: &SkilletConfig,
    workspace_path: &Path,
    ws: &ResolvedWorkspace,
    known_skills: &HashSet<String>,
    known_agents: &HashSet<String>,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let source_content = std::fs::read_to_string(&skill.source_path)
        .with_context(|| format!("failed to read {}", skill.source_path.display()))?;

    let known_files = ws.skill_files(skill);

    // Build diagnostic path using workspace + relative path with forward slashes.
    // This matches the PathBuf::join("src/skills/…") pattern used in tests, ensuring
    // consistent separator characters across OS (Windows preserves '/' in single-arg joins).
    let pan_path = workspace_path.join(format!(
        "{}/{}/{}.pan",
        cfg.workspace.skills_src_dir.trim_end_matches('/'),
        skill.name,
        skill.name
    ));
    let pan_source = PanSource::new(source_content, Some(pan_path));
    let ctx = CompileContext {
        source: pan_source,
        artifact_name: skill.name.clone(),
        fragments: &ws.rendered_fragments,
        vars: &cfg.vars,
        env: &cfg.env,
        known_files: &known_files,
        known_skills,
        known_commands: &ws.known_commands,
        known_agents,
        tokenizer: &cfg.build.tokenizer,
    };

    let result = compile_pan(&ctx)?;

    for w in &result.cmd_warnings {
        eprintln!("{}", w.render_text());
    }

    std::fs::create_dir_all(&skill.skill_out_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            skill.skill_out_dir.display()
        )
    })?;
    let output_path = skill.skill_out_dir.join("SKILL.md");
    std::fs::write(&output_path, &result.output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    copy_skill_subfolders(&skill.skill_dir, &skill.skill_out_dir)?;

    let source_hash = hash_file(&skill.source_path)?;
    let compiled_hash = hash_bytes(result.output.as_bytes());

    let old_minhash = lockfile
        .skills
        .get(&skill.name)
        .filter(|e| e.compiled_hash == compiled_hash)
        .map(|e| e.minhash.clone())
        .unwrap_or_default();

    let ref_tokens: u32 = result
        .ref_paths
        .iter()
        .filter_map(|rel| {
            let path = skill.skill_dir.join(rel);
            std::fs::read_to_string(&path)
                .ok()
                .map(|t| tokens::count_tokens(&t, &cfg.build.tokenizer))
        })
        .sum();
    let references_tokens: u32 = skill
        .references
        .iter()
        .filter_map(|r| {
            std::fs::read_to_string(&r.absolute_path)
                .ok()
                .map(|t| tokens::count_tokens(&t, &cfg.build.tokenizer))
        })
        .sum();
    let transitive_tokens = result.activation_tokens + ref_tokens + references_tokens;

    lockfile.skills.insert(
        skill.name.clone(),
        ArtefactEntry {
            source_hash,
            compiled_hash,
            discovery_tokens: result.discovery_tokens,
            activation_tokens: result.activation_tokens,
            transitive_tokens,
            fragments_used: result.fragments_used,
            refs: result.refs,
            minhash: old_minhash,
        },
    );

    Ok(())
}

fn rebuild_fragment_entries(lockfile: &mut Lockfile, ws: &ResolvedWorkspace) -> Result<()> {
    lockfile.fragments.clear();

    for (skill_name, entry) in &lockfile.skills {
        for frag_name in &entry.fragments_used {
            lockfile
                .fragments
                .entry(frag_name.clone())
                .or_insert_with(FragmentLockEntry::default)
                .used_by
                .push(skill_name.clone());
        }
    }

    for (frag_name, frag_entry) in &mut lockfile.fragments {
        if let Some(hash) = ws.fragment_hashes.get(frag_name) {
            frag_entry.hash = hash.clone();
            frag_entry.tokens = ws.fragment_tokens.get(frag_name).copied().unwrap_or(0);
        } else {
            let path = ws.root.join(format!("{}.fragment.pan", frag_name));
            if let Ok(h) = hash_file(&path) {
                frag_entry.hash = h;
            }
        }
        frag_entry.used_by.sort();
    }

    Ok(())
}

fn copy_skill_subfolders(skill_dir: &Path, skill_out_dir: &Path) -> Result<()> {
    for entry in WalkDir::new(skill_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sub_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let dest_sub_dir = skill_out_dir.join(&sub_name);
        if sub_name == "reference" {
            build_reference_dir(path, &dest_sub_dir)?;
        } else {
            skillet::workspace::copy_dir_recursive(path, &dest_sub_dir)?;
        }
    }
    Ok(())
}

fn build_reference_dir(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel == std::path::Path::new("") {
            continue;
        }
        if path.is_dir() {
            std::fs::create_dir_all(dest.join(rel))
                .with_context(|| format!("failed to create {}", dest.join(rel).display()))?;
        } else {
            let dest_file = if path.extension().and_then(|e| e.to_str()) == Some("pan") {
                dest.join(rel.with_extension("md"))
            } else {
                dest.join(rel)
            };
            if let Some(parent) = dest_file.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::copy(path, &dest_file).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    path.display(),
                    dest_file.display()
                )
            })?;
        }
    }
    Ok(())
}

fn verify_urls_from_lockfile(
    lockfile: &Lockfile,
    strict: bool,
    warnings: &mut Vec<String>,
    verbose: bool,
) -> Result<()> {
    use skillet::net::url_verify::{verify_urls, UrlCheckResult};

    let urls: Vec<String> = lockfile
        .skills
        .values()
        .flat_map(|e| e.refs.urls.iter().cloned())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if urls.is_empty() {
        return Ok(());
    }

    if verbose {
        println!("checking {} URL(s)…", urls.len());
    }
    let outcomes = verify_urls(&urls);

    let mut had_error = false;
    for outcome in &outcomes {
        match &outcome.result {
            UrlCheckResult::Ok => {}
            UrlCheckResult::Broken(code) => {
                let msg = format!("broken-url: {} ({})", outcome.url, code);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} ({})",
                        "warning[broken-url]:".yellow(),
                        outcome.url,
                        code
                    );
                }
                had_error = true;
            }
            UrlCheckResult::PossiblyDown(code) => {
                let msg = format!("url-possibly-down: {} ({})", outcome.url, code);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} ({})",
                        "info[url-possibly-down]:".cyan(),
                        outcome.url,
                        code
                    );
                }
            }
            UrlCheckResult::Unreachable(reason) => {
                let msg = format!("unreachable-url: {} — {}", outcome.url, reason);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} — {}",
                        "warning[unreachable-url]:".yellow(),
                        outcome.url,
                        reason
                    );
                }
                had_error = true;
            }
            UrlCheckResult::Rejected(reason) => {
                let msg = format!("rejected-url: {} — {}", outcome.url, reason);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} — {}",
                        "warning[rejected-url]:".yellow(),
                        outcome.url,
                        reason
                    );
                }
                had_error = true;
            }
        }
    }

    if strict && had_error {
        bail!("URL verification failed (--strict mode)");
    }

    Ok(())
}
