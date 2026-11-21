//! Freshness verification: compares workspace state against `skillet.lock`.
//!
//! This is the domain logic behind `skillet check` — a fast CI gate that never
//! recompiles. It hashes source files, on-disk `SKILL.md` and agent files, and fragments
//! and compares them against the hashes recorded in the lockfile, producing a
//! [`CheckReport`] ready for rendering.

use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::lockfile::{ArtefactEntry, Lockfile};
use crate::workspace::{self, Workspace};

/// Freshness verdict for one skill or agent.
#[derive(Debug, Serialize)]
pub struct ArtefactResult {
    /// Skill or agent name.
    pub name: String,
    /// Whether the artefact is up-to-date with the lockfile.
    pub fresh: bool,
    /// Human-readable reasons the artefact is stale (empty when fresh).
    pub reasons: Vec<String>,
    /// Machine-readable staleness differences.
    pub diffs: Vec<DiffEntry>,
}

/// A single machine-readable staleness difference.
#[derive(Debug, Serialize)]
pub struct DiffEntry {
    /// The kind of difference (e.g. `source_changed`, `fragment_missing`).
    pub kind: String,
    /// Optional file associated with the difference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Overall `check` output.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    /// Whether every skill and agent in the workspace is fresh.
    pub fresh: bool,
    /// Per-skill verdicts, sorted by name.
    pub skills: Vec<ArtefactResult>,
    /// Per-agent verdicts, sorted by name.
    pub agents: Vec<ArtefactResult>,
}

/// One compiled output file of an artefact, with the diff kinds and label used
/// when it is missing or changed.
struct Output {
    path: PathBuf,
    label: String,
    missing_kind: &'static str,
    changed_kind: &'static str,
}

/// Computes the freshness report by hashing sources/outputs/fragments against
/// the lockfile. The returned [`CheckReport`] is sorted and has `fresh`
/// computed, ready to render.
pub fn verify(ws: &Workspace, lockfile: &Lockfile) -> CheckReport {
    let mut skills: Vec<ArtefactResult> = ws
        .skills
        .values()
        .map(|skill| {
            let output = Output {
                path: skill.target_dir.join("SKILL.md"),
                label: "SKILL.md".to_string(),
                missing_kind: "skill_md_missing",
                changed_kind: "skill_md_changed",
            };
            verify_artefact(
                "skill",
                &skill.name,
                &skill.source_path,
                &[output],
                lockfile.skills.get(&skill.name),
            )
        })
        .collect();

    // Both agent copies are written from the same compiled text, so both are
    // compared against the one recorded compiled hash.
    let mut agents: Vec<ArtefactResult> = ws
        .agents
        .values()
        .map(|agent| {
            let outputs: Vec<Output> = [
                crate::agent::canonical_path(&agent.name),
                crate::agent::copilot_path(&agent.name),
            ]
            .into_iter()
            .map(|rel| Output {
                path: agent.out_dir.join(&rel),
                label: rel,
                missing_kind: "agent_output_missing",
                changed_kind: "agent_output_changed",
            })
            .collect();
            verify_artefact(
                "agent",
                &agent.name,
                &agent.source_path,
                &outputs,
                lockfile.agents.get(&agent.name),
            )
        })
        .collect();

    // Verify fragment hashes against lockfile.
    for (frag_name, frag_entry) in &lockfile.fragments {
        let frag_path_opt = ws.fragment_paths.get(frag_name);
        let (reason, diff_kind, file_str) = match frag_path_opt {
            None => (
                Some(format!(
                    "fragment '{frag_name}' is missing — run `skillet build`"
                )),
                Some("fragment_missing"),
                None,
            ),
            Some(frag_path) => match workspace::hash_file(frag_path) {
                Err(_) => (
                    Some(format!(
                        "fragment '{frag_name}' is missing — run `skillet build`"
                    )),
                    Some("fragment_missing"),
                    Some(frag_path.to_string_lossy().to_string()),
                ),
                Ok(current_hash) if current_hash != frag_entry.hash => (
                    Some(format!(
                        "fragment '{frag_name}' has changed since last build — run `skillet build`"
                    )),
                    Some("fragment_changed"),
                    Some(frag_path.to_string_lossy().to_string()),
                ),
                Ok(_) => (None, None, None),
            },
        };
        if let (Some(r), Some(dk)) = (reason, diff_kind) {
            for user in &frag_entry.used_by {
                for result in skills
                    .iter_mut()
                    .chain(agents.iter_mut())
                    .filter(|r| &r.name == user)
                {
                    result.fresh = false;
                    result.reasons.push(r.clone());
                    result.diffs.push(DiffEntry {
                        kind: dk.to_string(),
                        file: file_str.clone(),
                    });
                }
            }
        }
    }

    // Flag lockfile entries whose source no longer exists.
    let skill_names: HashSet<&str> = ws.skills.keys().map(String::as_str).collect();
    for locked_name in lockfile.skills.keys() {
        if !skill_names.contains(locked_name.as_str()) {
            skills.push(orphan(
                format!(
                    "skill '{locked_name}' is in lockfile but source directory no longer exists \
                     — run `skillet build`"
                ),
                locked_name,
                "source_dir_missing",
            ));
        }
    }
    for locked_name in lockfile.agents.keys() {
        if !ws.agents.contains_key(locked_name) {
            agents.push(orphan(
                format!(
                    "agent '{locked_name}' is in lockfile but its source no longer exists \
                     — run `skillet build`"
                ),
                locked_name,
                "source_missing",
            ));
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    agents.sort_by(|a, b| a.name.cmp(&b.name));

    let fresh = skills.iter().chain(&agents).all(|r| r.fresh);
    CheckReport {
        fresh,
        skills,
        agents,
    }
}

fn orphan(reason: String, name: &str, kind: &str) -> ArtefactResult {
    ArtefactResult {
        name: name.to_string(),
        fresh: false,
        reasons: vec![reason],
        diffs: vec![DiffEntry {
            kind: kind.to_string(),
            file: None,
        }],
    }
}

/// Compares one artefact's source and output files against its lockfile entry.
fn verify_artefact(
    kind: &str,
    name: &str,
    source_path: &Path,
    outputs: &[Output],
    entry: Option<&ArtefactEntry>,
) -> ArtefactResult {
    let mut reasons: Vec<String> = Vec::new();
    let mut diffs: Vec<DiffEntry> = Vec::new();

    match entry {
        None => {
            reasons.push(format!(
                "{kind} '{name}' not in lockfile — run `skillet build`"
            ));
            diffs.push(DiffEntry {
                kind: "not_in_lockfile".to_string(),
                file: None,
            });
        }
        Some(entry) => {
            match workspace::hash_file(source_path) {
                Err(e) => reasons.push(format!("could not hash source: {}", e)),
                Ok(hash) if hash != entry.source_hash => {
                    let fname = source_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    reasons.push(format!("source '{}' has changed since last build", fname));
                    diffs.push(DiffEntry {
                        kind: "source_changed".to_string(),
                        file: Some(source_path.to_string_lossy().to_string()),
                    });
                }
                Ok(_) => {}
            }

            for output in outputs {
                let file = Some(output.path.to_string_lossy().to_string());
                if !output.path.exists() {
                    reasons.push(format!("{} is missing — run `skillet build`", output.label));
                    diffs.push(DiffEntry {
                        kind: output.missing_kind.to_string(),
                        file,
                    });
                    continue;
                }
                match workspace::hash_file(&output.path) {
                    Err(e) => reasons.push(format!("could not hash {}: {}", output.label, e)),
                    Ok(hash) if hash != entry.compiled_hash => {
                        reasons.push(format!(
                            "{} does not match last build — run `skillet build`",
                            output.label
                        ));
                        diffs.push(DiffEntry {
                            kind: output.changed_kind.to_string(),
                            file,
                        });
                    }
                    Ok(_) => {}
                }
            }
        }
    }

    ArtefactResult {
        name: name.to_string(),
        fresh: reasons.is_empty(),
        reasons,
        diffs,
    }
}
