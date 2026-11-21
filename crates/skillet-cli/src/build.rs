//! Build orchestration: resolves the workspace, compiles skills, writes outputs,
//! and updates `skillet.lock`.
//!
//! All filesystem I/O for the build pipeline lives here. The pure compilation
//! logic is delegated to `skillet::compiler::compile_pan()`.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use owo_colors::OwoColorize;
use serde::Serialize;
use skillet::agent::AgentDefinition;
use skillet::compiler::check::{check_source_file, CheckDiag, Severity};
use skillet::compiler::PanSource;
use skillet::config::SkilletConfig;
use skillet::lockfile::{self, ArtefactEntry, ArtefactRefs, LockMeta, Lockfile};
use skillet::workspace::{hash_bytes, hash_file, Agent, Skill, Workspace};
use std::path::Path;

// ── Build options ──────────────────────────────────────────────────────────────

/// Output format for build results.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    /// Suppresses all stdout output (used when `publish --format json` invokes a pre-build).
    Quiet,
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
    /// Names of agents that were built during this run.
    pub agents_built: Vec<String>,
    /// Warnings generated during the build (e.g. missing commands or broken URLs).
    pub warnings: Vec<String>,
    /// Path to the lockfile used during the build.
    pub lockfile_path: String,
}

/// Compiles `.pan` sources to `SKILL.md` files and agent sources to agent
/// definitions, then updates `skillet.lock`.
pub fn run(
    workspace_path: &Path,
    skill_name: Option<&str>,
    plugin_name: Option<&str>,
    opts: &BuildOptions,
    cfg: &SkilletConfig,
) -> Result<()> {
    let ws = Workspace::resolve(workspace_path, cfg)?;

    if let Some(plugin_name) = plugin_name {
        if !cfg.plugins.contains_key(plugin_name) {
            bail!("plugin '{}' not found in skillet.toml", plugin_name);
        }
    }

    let targets: Vec<&Skill> = match skill_name {
        Some(name) => match ws.skills.get(name) {
            Some(s) => vec![s],
            None => bail!("skill '{}' not found in workspace", name),
        },
        None => ws
            .skills
            .values()
            .filter(|s| plugin_name.is_none_or(|p| s.plugin == p))
            .collect(),
    };

    // A single-skill build does not build agents.
    let mut agent_targets: Vec<&Agent> = match skill_name {
        Some(_) => vec![],
        None => ws
            .agents
            .values()
            .filter(|a| plugin_name.is_none_or(|p| a.plugin == p))
            .collect(),
    };
    agent_targets.sort_by(|a, b| a.name.cmp(&b.name));

    if targets.is_empty() && agent_targets.is_empty() {
        if opts.format == OutputFormat::Json {
            let report = BuildReport {
                skills_built: vec![],
                agents_built: vec![],
                warnings: vec![],
                lockfile_path: workspace_path
                    .join("skillet.lock")
                    .to_string_lossy()
                    .to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            eprintln!("no skills or agents found");
        }
        return Ok(());
    }

    // Detect output collisions before writing anything.
    {
        let mut seen: std::collections::HashMap<&std::path::Path, (&str, &str)> =
            std::collections::HashMap::new();
        for skill in &targets {
            if let Some((other_plugin, other_skill)) =
                seen.insert(skill.target_dir.as_path(), (&skill.plugin, &skill.name))
            {
                bail!(
                    "output collision: skill '{}' (plugin '{}') and skill '{}' (plugin '{}') \
                     both write to {}",
                    skill.name,
                    skill.plugin,
                    other_skill,
                    other_plugin,
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
        if opts.format == OutputFormat::Text {
            println!("built {}", skill.name);
        }
        skills_built.push(skill.name.clone());
    }

    let mut agents_built: Vec<String> = Vec::new();
    for agent in &agent_targets {
        compile_one_agent(agent, &ws, &mut lockfile)?;
        if opts.format == OutputFormat::Text {
            println!("built agent {}", agent.name);
        }
        agents_built.push(agent.name.clone());
    }

    rebuild_fragment_entries(&mut lockfile, &ws)?;

    let lock_path = workspace_path.join("skillet.lock");
    lockfile::write(workspace_path, &lockfile)?;

    if cfg.build.verify_urls && !opts.offline {
        verify_urls_from_lockfile(
            &lockfile,
            opts.strict,
            &mut warnings,
            opts.format == OutputFormat::Text,
        )?;
    }

    if opts.format == OutputFormat::Json {
        let report = BuildReport {
            skills_built,
            agents_built,
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

    let diags = check_source_file(ws, &pan_source, &skill.references);

    for d in &diags {
        eprintln!("{}", render_diag(&skill.name, &skill.source_path, d));
    }

    if diags.iter().any(|d| d.severity == Severity::Error) {
        anyhow::bail!("check errors in '{}'", skill.name);
    }

    let output = skillet::compiler::compile::compile(ws, &pan_source, &skill.references);

    std::fs::create_dir_all(&skill.target_dir)?;
    let output_path = skill.target_dir.join("SKILL.md");
    std::fs::write(&output_path, &output.text)?;

    copy_skill_child_files(skill)?;

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
            transitive_tokens: skillet::budget::transitive_tokens(
                output.activation_tokens,
                &skill.references,
                &ws.tokenizer,
            ),
            fragments_used: output.fragments_used,
            refs,
            minhash: old_minhash,
        },
    );

    Ok(())
}

/// Copies a skill's `references/`, `scripts/`, and `tests/` directories into its
/// target directory, recursively and 1-to-1, preserving the source structure.
fn copy_skill_child_files(skill: &Skill) -> Result<()> {
    for sub in ["references", "scripts", "tests"] {
        let src_root = skill.src_dir.join(sub);
        if !src_root.is_dir() {
            continue;
        }
        let dst_root = skill.target_dir.join(sub);

        for entry in walkdir::WalkDir::new(&src_root)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let rel = entry
                .path()
                .strip_prefix(&src_root)
                .expect("walkdir entry is under src_root");
            let dst = dst_root.join(rel);

            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&dst)
                    .with_context(|| format!("failed to create {}", dst.display()))?;
            } else if entry.file_type().is_file() {
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create {}", parent.display()))?;
                }
                std::fs::copy(entry.path(), &dst).with_context(|| {
                    format!(
                        "failed to copy {} to {}",
                        entry.path().display(),
                        dst.display()
                    )
                })?;
            }
        }
    }

    Ok(())
}

/// Compiles one agent source, validates it as an agent definition, and writes
/// the canonical file plus the GitHub Copilot copy.
fn compile_one_agent(agent: &Agent, ws: &Workspace, lockfile: &mut Lockfile) -> Result<()> {
    let source_content = std::fs::read_to_string(&agent.source_path)
        .with_context(|| format!("failed to read {}", agent.source_path.display()))?;
    let pan_source = PanSource::new(source_content);

    // An agent has no `references/` directory, so every `ref::` is reported.
    let diags = check_source_file(ws, &pan_source, &[]);
    for d in &diags {
        eprintln!("{}", render_diag(&agent.name, &agent.source_path, d));
    }
    if diags.iter().any(|d| d.severity == Severity::Error) {
        bail!("check errors in agent '{}'", agent.name);
    }

    let output = skillet::compiler::compile::compile(ws, &pan_source, &[]);

    let definition = AgentDefinition::parse(&output.text, &agent.name).map_err(|e| {
        anyhow::anyhow!(
            "[agent-{}] {} ({})",
            e.code(),
            e,
            agent.source_path.display()
        )
    })?;

    for rel in [
        skillet::agent::canonical_path(definition.name.as_str()),
        skillet::agent::copilot_path(definition.name.as_str()),
    ] {
        let path = agent.out_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&path, &output.text)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }

    let skill_names: Vec<&str> = ws.skills.keys().map(String::as_str).collect();
    let parsed_refs = skillet::refs::ParsedRefs::extract(pan_source.as_str(), &skill_names);

    lockfile.agents.insert(
        agent.name.clone(),
        ArtefactEntry {
            source_hash: hash_file(&agent.source_path)?,
            compiled_hash: hash_bytes(output.text.as_bytes()),
            discovery_tokens: output.discovery_tokens,
            activation_tokens: output.activation_tokens,
            // An agent has no referenced files: its transitive cost is its activation cost.
            transitive_tokens: output.activation_tokens,
            fragments_used: output.fragments_used,
            refs: ArtefactRefs::from_parsed(&parsed_refs),
            minhash: vec![],
        },
    );

    Ok(())
}

fn render_diag(name: &str, source_path: &Path, d: &CheckDiag) -> String {
    let level = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    format!(
        "[{level}] {} {} ({}:{}:{})",
        name,
        d.message,
        source_path.display(),
        d.line,
        d.col
    )
}

fn rebuild_fragment_entries(lockfile: &mut Lockfile, ws: &Workspace) -> Result<()> {
    lockfile.fragments.clear();

    for (skill_name, entry) in lockfile.skills.iter().chain(&lockfile.agents) {
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
