//! Freshness verification: compares workspace state against `skillet.lock`.
//!
//! `skillet check` is designed as a fast CI gate — it never recompiles; it
//! just hashes source files and on-disk `SKILL.md` files and compares them
//! against the recorded hashes in the lockfile.

use anyhow::{Context, Result};
use serde::Serialize;
use skillet::config::SkilletConfig;
use skillet::lockfile;
use skillet::workspace::{self, Workspace};
use std::path::Path;

/// How results are rendered.
#[derive(Clone, Copy, Default)]
pub enum OutputFormat {
    /// Default text output.
    #[default]
    Text,
    /// Machine-parseable JSON output.
    Json,
}

/// Freshness verdict for one skill.
#[derive(Debug, Serialize)]
pub struct SkillResult {
    pub name: String,
    pub fresh: bool,
    pub reasons: Vec<String>,
    pub diffs: Vec<DiffEntry>,
}

/// A single machine-readable staleness difference.
#[derive(Debug, Serialize)]
pub struct DiffEntry {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Overall `check` output.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub fresh: bool,
    pub skills: Vec<SkillResult>,
}

/// Runs freshness checks for all skills in the workspace.
pub fn run(workspace_path: &Path, format: OutputFormat, config: &SkilletConfig) -> Result<bool> {
    let ws = Workspace::resolve(workspace_path, config)?;
    let fragments_dir = workspace_path.join(&config.workspace.fragments_dir);

    let lock_path = workspace_path.join("skillet.lock");
    if !lock_path.exists() {
        anyhow::bail!("skillet.lock not found — run `skillet build` to generate it");
    }
    let lockfile = lockfile::read(workspace_path)
        .with_context(|| format!("failed to read {}", lock_path.display()))?;

    let source_names: std::collections::HashSet<&str> =
        ws.skills.iter().map(|s| s.name.as_str()).collect();

    let mut results: Vec<SkillResult> = ws
        .skills
        .iter()
        .map(|skill| {
            let mut reasons: Vec<String> = Vec::new();
            let mut diffs: Vec<DiffEntry> = Vec::new();

            match lockfile.skills.get(&skill.name) {
                None => {
                    reasons.push(format!(
                        "skill '{}' not in lockfile — run `skillet build`",
                        skill.name
                    ));
                    diffs.push(DiffEntry {
                        kind: "not_in_lockfile".to_string(),
                        file: None,
                    });
                }
                Some(entry) => {
                    match workspace::hash_file(&skill.source_path) {
                        Err(e) => reasons.push(format!("could not hash source: {}", e)),
                        Ok(current_source_hash) => {
                            if current_source_hash != entry.source_hash {
                                let fname = skill
                                    .source_path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string();
                                reasons.push(format!(
                                    "source '{}' has changed since last build",
                                    fname
                                ));
                                diffs.push(DiffEntry {
                                    kind: "source_changed".to_string(),
                                    file: Some(skill.source_path.to_string_lossy().to_string()),
                                });
                            }
                        }
                    }

                    let skill_md = skill.skill_out_dir.join("SKILL.md");
                    if !skill_md.exists() {
                        reasons.push("SKILL.md is missing — run `skillet build`".to_string());
                        diffs.push(DiffEntry {
                            kind: "skill_md_missing".to_string(),
                            file: Some(skill_md.to_string_lossy().to_string()),
                        });
                    } else {
                        match workspace::hash_file(&skill_md) {
                            Err(e) => reasons.push(format!("could not hash SKILL.md: {}", e)),
                            Ok(current_compiled_hash) => {
                                if current_compiled_hash != entry.compiled_hash {
                                    reasons.push(
                                        "SKILL.md does not match last build — run `skillet build`"
                                            .to_string(),
                                    );
                                    diffs.push(DiffEntry {
                                        kind: "skill_md_changed".to_string(),
                                        file: Some(skill_md.to_string_lossy().to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            let fresh = reasons.is_empty();
            SkillResult {
                name: skill.name.clone(),
                fresh,
                reasons,
                diffs,
            }
        })
        .collect();

    // Verify fragment hashes against lockfile.
    for (frag_name, frag_entry) in &lockfile.fragments {
        let frag_path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        let (reason, diff_kind) = match workspace::hash_file(&frag_path) {
            Err(_) => (
                Some(format!(
                    "fragment '{frag_name}' is missing — run `skillet build`"
                )),
                Some("fragment_missing"),
            ),
            Ok(current_hash) if current_hash != frag_entry.hash => (
                Some(format!(
                    "fragment '{frag_name}' has changed since last build — run `skillet build`"
                )),
                Some("fragment_changed"),
            ),
            Ok(_) => (None, None),
        };
        if let (Some(r), Some(dk)) = (reason, diff_kind) {
            for skill_name in &frag_entry.used_by {
                if let Some(result) = results.iter_mut().find(|s| &s.name == skill_name) {
                    result.fresh = false;
                    result.reasons.push(r.clone());
                    result.diffs.push(DiffEntry {
                        kind: dk.to_string(),
                        file: Some(frag_path.to_string_lossy().to_string()),
                    });
                }
            }
        }
    }

    // Flag lockfile entries whose source directory no longer exists.
    for locked_name in lockfile.skills.keys() {
        if !source_names.contains(locked_name.as_str()) {
            results.push(SkillResult {
                name: locked_name.clone(),
                fresh: false,
                reasons: vec![format!(
                    "skill '{}' is in lockfile but source directory no longer exists \
                     — run `skillet build`",
                    locked_name
                )],
                diffs: vec![DiffEntry {
                    kind: "source_dir_missing".to_string(),
                    file: None,
                }],
            });
        }
    }

    results.sort_by(|a, b| a.name.cmp(&b.name));

    let all_fresh = results.iter().all(|r| r.fresh);
    let report = CheckReport {
        fresh: all_fresh,
        skills: results,
    };

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Text => render(&report),
    }

    Ok(all_fresh)
}

fn render(report: &CheckReport) {
    let stale: Vec<&SkillResult> = report.skills.iter().filter(|r| !r.fresh).collect();

    if stale.is_empty() {
        let n = report.skills.len();
        println!(
            "✓ all {} skill{} up-to-date",
            n,
            if n == 1 { " is" } else { "s are" }
        );
        return;
    }

    for skill in &stale {
        for reason in &skill.reasons {
            println!("✗ {}: {}", skill.name, reason);
        }
    }
    println!(
        "\n{} skill{} stale",
        stale.len(),
        if stale.len() == 1 { " is" } else { "s are" }
    );
}
