//! Build orchestration: resolves the workspace, compiles skills, writes outputs,
//! and updates `skillet.lock`.
//!
//! All filesystem I/O for the build pipeline lives here. The pure compilation
//! logic is delegated to `skillet::compiler::compile_pan()`.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use owo_colors::OwoColorize;
use serde::Serialize;
use skillet::compiler::check::{check_source_file, CheckDiag, Severity};
use skillet::compiler::PanSource;
use skillet::config::SkilletConfig;
use skillet::lockfile::{self, ArtefactEntry, ArtefactRefs, LockMeta, Lockfile};
use skillet::workspace::{hash_bytes, hash_file, Skill, Workspace};
use std::path::Path;

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
    module_name: Option<&str>,
    opts: &BuildOptions,
    cfg: &SkilletConfig,
) -> Result<()> {
    let ws = Workspace::resolve(workspace_path, cfg)?;

    let targets: Vec<&Skill> = match skill_name {
        Some(name) => match ws.skills.get(name) {
            Some(s) => vec![s],
            None => bail!("skill '{}' not found in workspace", name),
        },
        None => {
            if let Some(mod_name) = module_name {
                let skills: Vec<&Skill> = ws
                    .skills
                    .values()
                    .filter(|s| s.module == mod_name)
                    .collect();
                if skills.is_empty() {
                    bail!("module '{}' not found or has no skills", mod_name);
                }
                skills
            } else {
                ws.skills.values().collect()
            }
        }
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
            eprintln!("no skills found");
        }
        return Ok(());
    }

    // Detect output collisions before writing anything.
    {
        let mut seen: std::collections::HashMap<&std::path::Path, (&str, &str)> =
            std::collections::HashMap::new();
        for skill in &targets {
            if let Some((other_module, other_skill)) =
                seen.insert(skill.target_dir.as_path(), (&skill.module, &skill.name))
            {
                bail!(
                    "output collision: skill '{}' (module '{}') and skill '{}' (module '{}') \
                     both write to {}",
                    skill.name,
                    skill.module,
                    other_skill,
                    other_module,
                    skill.target_dir.display()
                );
            }
        }
    }

    let mut lockfile = lockfile::read(workspace_path)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: cfg.build.tokenizer.clone(),
    });

    let mut skills_built: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for skill in &targets {
        compile_one_skill(skill, &ws, &mut lockfile)?;
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

fn compile_one_skill(skill: &Skill, ws: &Workspace, lockfile: &mut Lockfile) -> Result<()> {
    let source_content = std::fs::read_to_string(&skill.source_path)
        .with_context(|| format!("failed to read {}", skill.source_path.display()))?;

    let pan_source = PanSource::new(source_content);

    let known_files = ws.get_source_files_for_skill(skill);

    let diags = check_source_file(ws, &pan_source, &known_files);

    for d in &diags {
        eprintln!("{}", render_diag(skill, d));
    }

    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("check errors in '{}'", skill.name);
    }

    let output = skillet::compiler::compile::compile(ws, &pan_source);

    std::fs::create_dir_all(&skill.target_dir)?;
    let output_path = skill.target_dir.join("SKILL.md");
    std::fs::write(&output_path, &output.text)?;

    let source_hash = hash_file(&skill.source_path)?;
    let compiled_hash = hash_bytes(output.text.as_bytes());

    let skill_names: Vec<&str> = ws.skills.keys().map(String::as_str).collect();
    let parsed_refs = skillet::refs::ParsedRefs::extract(pan_source.as_str(), &skill_names);

    let refs = ArtefactRefs::from_parsed(&parsed_refs);

    let old_minhash = lockfile
        .skills
        .get(&skill.name)
        .filter(|e| e.compiled_hash == compiled_hash)
        .map(|e| e.minhash.clone())
        .unwrap_or_default();

    lockfile.skills.insert(
        skill.name.clone(),
        ArtefactEntry {
            source_hash,
            compiled_hash,
            discovery_tokens: output.discovery_tokens,
            activation_tokens: output.activation_tokens,
            transitive_tokens: 0,
            fragments_used: output.fragments_used,
            refs,
            minhash: old_minhash,
        },
    );

    Ok(())
}

fn render_diag(skill: &Skill, d: &CheckDiag) -> String {
    let level = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    format!(
        "[{level}] {} {} ({}:{}:{})",
        skill.name,
        d.message,
        skill.source_path.display(),
        d.line,
        d.col
    )
}

fn rebuild_fragment_entries(lockfile: &mut Lockfile, ws: &Workspace) -> Result<()> {
    lockfile.fragments.clear();

    for (skill_name, entry) in &lockfile.skills {
        for frag_name in &entry.fragments_used {
            lockfile
                .fragments
                .entry(frag_name.clone())
                .or_default()
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
