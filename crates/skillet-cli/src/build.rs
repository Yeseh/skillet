//! Build orchestration: discovers skills, compiles them, writes outputs, and
//! updates `skillet.lock`.
//!
//! All filesystem I/O for the build pipeline lives here. The pure compilation
//! logic is delegated to `skillet::compile::compile()`.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use owo_colors::OwoColorize;
use serde::Serialize;
use sha2::Digest;
use skillet::compile::{self, BuildOptions, CompileContext, OutputFormat, SourceUnit};
use skillet::config::SkilletConfig;
use skillet::lockfile::{self, FragmentLockEntry, LockMeta, Lockfile, SkillEntry};
use skillet::tokens;
use skillet::workspace::SkillSource;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use walkdir::WalkDir;

use crate::workspace::{self as cli_workspace, Skill};

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
    workspace: &Path,
    skill_name: Option<&str>,
    opts: &BuildOptions,
    cfg: &SkilletConfig,
) -> Result<()> {
    let skills_src_dir = workspace.join(&cfg.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&cfg.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&cfg.workspace.fragments_dir);

    let sources = skillet::workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;
    let agents_dir = workspace.join("agents");
    let ws = cli_workspace::resolve(workspace, &skills_src_dir, &agents_dir)?;
    let skill_map: HashMap<&str, &Skill> = ws.skills.iter().map(|s| (s.name.as_str(), s)).collect();

    let targets: Vec<&SkillSource> = match skill_name {
        Some(name) => {
            let found = sources.iter().find(|s| s.name == name);
            match found {
                Some(s) => vec![s],
                None => bail!("skill '{}' not found in workspace", name),
            }
        }
        None => sources.iter().collect(),
    };

    if targets.is_empty() {
        if opts.format == OutputFormat::Json {
            let report = BuildReport {
                skills_built: vec![],
                warnings: vec![],
                lockfile_path: workspace.join("skillet.lock").to_string_lossy().to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            eprintln!("no skills found in {}", skills_src_dir.display());
        }
        return Ok(());
    }

    let fragments = load_all_fragments(&fragments_dir)?;
    let known_skills: HashSet<String> = sources.iter().map(|s| s.name.clone()).collect();

    let mut lockfile = lockfile::read(workspace)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: cfg.build.tokenizer.clone(),
    });

    let mut skills_built: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for source in &targets {
        let skill = skill_map.get(source.name.as_str()).copied();
        compile_one_skill(source, skill, cfg, &fragments, &known_skills, &mut lockfile)?;
        if opts.format != OutputFormat::Json {
            println!("built {}", source.name);
        }
        skills_built.push(source.name.clone());
    }

    rebuild_fragment_entries(&mut lockfile, &fragments_dir, &cfg.build.tokenizer)?;

    let lock_path = workspace.join("skillet.lock");
    lockfile::write(workspace, &lockfile)?;

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

fn load_all_fragments(fragments_dir: &Path) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    if !fragments_dir.exists() {
        return Ok(map);
    }
    for entry in WalkDir::new(fragments_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".fragment.pan"))
        {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read fragment '{}'", path.display()))?;
            map.insert(name.to_string(), content);
        }
    }
    Ok(map)
}

fn compile_one_skill(
    source: &SkillSource,
    skill: Option<&Skill>,
    cfg: &SkilletConfig,
    fragments: &HashMap<String, String>,
    known_skills: &HashSet<String>,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let source_content = std::fs::read_to_string(&source.source_path)
        .with_context(|| format!("failed to read {}", source.source_path.display()))?;

    let known_files: HashSet<String> = WalkDir::new(&source.skill_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            e.path()
                .strip_prefix(&source.skill_dir)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .collect();

    let ctx = CompileContext {
        source: SourceUnit {
            name: source.name.clone(),
            path: source.source_path.to_string_lossy().to_string(),
            content: source_content.clone(),
        },
        fragments: fragments.clone(),
        known_files,
        known_commands: HashSet::new(),
        known_skills: known_skills.clone(),
        vars: cfg.vars.clone(),
        env: cfg.env.clone(),
        tokenizer: cfg.build.tokenizer.clone(),
    };

    let result = compile::compile(&ctx)?;

    for w in &result.cmd_warnings {
        eprintln!("{}", w.render_text());
    }

    std::fs::create_dir_all(&source.skill_out_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            source.skill_out_dir.display()
        )
    })?;
    let output_path = source.skill_out_dir.join("SKILL.md");
    std::fs::write(&output_path, &result.output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    copy_skill_subfolders(&source.skill_dir, &source.skill_out_dir)?;

    let source_hash = hash_file(&source.source_path)?;
    let compiled_hash = hash_bytes(result.output.as_bytes());

    let old_minhash = lockfile
        .skills
        .get(&source.name)
        .filter(|e| e.compiled_hash == compiled_hash)
        .map(|e| e.minhash.clone())
        .unwrap_or_default();

    let ref_tokens: u32 = result
        .ref_paths
        .iter()
        .filter_map(|rel| {
            let path = source.skill_dir.join(rel);
            std::fs::read_to_string(&path)
                .ok()
                .map(|t| tokens::count_tokens(&t, &cfg.build.tokenizer))
        })
        .sum();
    let references_tokens: u32 = skill
        .map(|s| {
            s.references
                .iter()
                .filter_map(|r| {
                    std::fs::read_to_string(&r.absolute_path)
                        .ok()
                        .map(|t| tokens::count_tokens(&t, &cfg.build.tokenizer))
                })
                .sum()
        })
        .unwrap_or(0);
    let transitive_tokens = result.activation_tokens + ref_tokens + references_tokens;

    lockfile.skills.insert(
        source.name.clone(),
        SkillEntry {
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

fn rebuild_fragment_entries(
    lockfile: &mut Lockfile,
    fragments_dir: &Path,
    tokenizer: &str,
) -> Result<()> {
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
        let path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        if let Ok(text) = std::fs::read_to_string(&path) {
            frag_entry.hash = hash_bytes(text.as_bytes());
            frag_entry.tokens = tokens::count_tokens(&text, tokenizer);
        } else if let Ok(h) = hash_file(&path) {
            frag_entry.hash = h;
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

// ── Hashing helpers ────────────────────────────────────────────────────────────

/// Returns `"sha256:<hex>"` of the file at `path`.
pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {} for hashing", path.display()))?;
    Ok(format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(&bytes))
    ))
}

/// Returns `"sha256:<hex>"` of `bytes` (in-memory hashing).
pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
}
