//! Test-only build helper that performs the full compile-and-write cycle.
//!
//! This module is **not** part of the public API — it exists solely to let
//! internal integration tests (`check`, `budget`, `lint`) set up a built
//! workspace without depending on the CLI crate.

use crate::compile::{self, CompileContext, SourceUnit};
use crate::config::SkilletConfig;
use crate::lockfile::{FragmentLockEntry, LockMeta, Lockfile, SkillEntry};
use crate::workspace::{self, SkillSource};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use walkdir::WalkDir;

/// Compiles `.pan` sources to `SKILL.md` files and updates `skillet.lock`.
pub fn build_workspace(
    workspace: &Path,
    skill_name: Option<&str>,
    cfg: &SkilletConfig,
) -> Result<()> {
    let skills_src_dir = workspace.join(&cfg.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&cfg.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&cfg.workspace.fragments_dir);

    let sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;

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
        return Ok(());
    }

    let fragments = load_all_fragments(&fragments_dir)?;
    let known_skills: HashSet<String> = sources.iter().map(|s| s.name.clone()).collect();

    let mut lockfile = crate::lockfile::read(workspace)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: cfg.build.tokenizer.clone(),
    });

    for source in &targets {
        compile_one_skill(source, cfg, &fragments, &known_skills, &mut lockfile)?;
    }

    rebuild_fragment_entries(&mut lockfile, &fragments_dir, &cfg.build.tokenizer)?;
    crate::lockfile::write(workspace, &lockfile)?;

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

    std::fs::create_dir_all(&source.skill_out_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            source.skill_out_dir.display()
        )
    })?;
    let output_path = source.skill_out_dir.join("SKILL.md");
    std::fs::write(&output_path, &result.output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    workspace::copy_dir_recursive(&source.skill_dir, &source.skill_out_dir)?;

    let source_hash = workspace::hash_file(&source.source_path)?;
    let compiled_hash = compile::hash_bytes(result.output.as_bytes());

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
                .map(|t| crate::tokens::count_tokens(&t, &cfg.build.tokenizer))
        })
        .sum();
    let references_dir = source.skill_dir.join("references");
    let references_tokens: u32 = if references_dir.is_dir() {
        walkdir::WalkDir::new(&references_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("pan")
            })
            .filter_map(|e| {
                std::fs::read_to_string(e.path())
                    .ok()
                    .map(|t| crate::tokens::count_tokens(&t, &cfg.build.tokenizer))
            })
            .sum()
    } else {
        0
    };
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
            frag_entry.hash = compile::hash_bytes(text.as_bytes());
            frag_entry.tokens = crate::tokens::count_tokens(&text, tokenizer);
        } else if let Ok(h) = workspace::hash_file(&path) {
            frag_entry.hash = h;
        }
        frag_entry.used_by.sort();
    }

    Ok(())
}
