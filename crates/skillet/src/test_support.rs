//! Test-only build helper that performs the full compile-and-write cycle.
//!
//! This module is **not** part of the public API — it exists solely to let
//! internal integration tests (`check`, `budget`, `lint`) set up a built
//! workspace without depending on the CLI crate.

use crate::compiler::PanSource;
use crate::compiler::{compile_pan, CompileContext};
use crate::config::SkilletConfig;
use crate::lockfile::{ArtefactEntry, FragmentLockEntry, LockMeta, Lockfile};
use crate::workspace::{self, hash_bytes, Skill, Workspace};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::path::Path;

/// Compiles `.pan` sources to `SKILL.md` files and updates `skillet.lock`.
pub fn build_workspace(
    workspace_path: &Path,
    skill_name: Option<&str>,
    cfg: &SkilletConfig,
) -> Result<()> {
    let ws = Workspace::resolve(workspace_path, cfg)?;

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

    let mut lockfile = crate::lockfile::read(workspace_path)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: cfg.build.tokenizer.clone(),
    });

    for skill in &targets {
        compile_one_skill(skill, cfg, &ws, &known_skills, &known_agents, &mut lockfile)?;
    }

    rebuild_fragment_entries(&mut lockfile, &ws)?;
    crate::lockfile::write(workspace_path, &lockfile)?;

    Ok(())
}

fn compile_one_skill(
    skill: &Skill,
    cfg: &SkilletConfig,
    ws: &Workspace,
    known_skills: &HashSet<String>,
    known_agents: &HashSet<String>,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let source_content = std::fs::read_to_string(&skill.source_path)
        .with_context(|| format!("failed to read {}", skill.source_path.display()))?;

    let known_files = ws.skill_files(skill);
    let known_commands = HashSet::new();

    let pan_source = PanSource::new(source_content, Some(skill.source_path.clone()));
    let ctx = CompileContext {
        source: pan_source,
        artifact_name: skill.name.clone(),
        fragments: &ws.rendered_fragments,
        vars: &cfg.vars,
        env: &cfg.env,
        known_files: &known_files,
        known_skills,
        known_commands: &known_commands,
        known_agents,
        tokenizer: &cfg.build.tokenizer,
    };

    let result = compile_pan(&ctx).map_err(|f| anyhow::anyhow!("{}", f))?;

    std::fs::create_dir_all(&skill.skill_out_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            skill.skill_out_dir.display()
        )
    })?;
    let output_path = skill.skill_out_dir.join("SKILL.md");
    std::fs::write(&output_path, &result.output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    workspace::copy_dir_recursive(&skill.skill_dir, &skill.skill_out_dir)?;

    let source_hash = workspace::hash_file(&skill.source_path)?;
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
                .map(|t| crate::tokens::count_tokens(&t, &cfg.build.tokenizer))
        })
        .sum();
    let references_dir = skill.skill_dir.join("references");
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

fn rebuild_fragment_entries(lockfile: &mut Lockfile, ws: &Workspace) -> Result<()> {
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
            if let Ok(h) = workspace::hash_file(&path) {
                frag_entry.hash = h;
            }
        }
        frag_entry.used_by.sort();
    }

    Ok(())
}
