//! Freshness verification: compares workspace state against `skillet.lock`.
//!
//! This is the domain logic behind `skillet check` — a fast CI gate that never
//! recompiles. It hashes source files, on-disk `SKILL.md` files, and fragments
//! and compares them against the hashes recorded in the lockfile, producing a
//! [`CheckReport`] ready for rendering.

use serde::Serialize;

use crate::lockfile::Lockfile;
use crate::workspace::{self, Workspace};

/// Freshness verdict for one skill.
#[derive(Debug, Serialize)]
pub struct SkillResult {
    /// Skill name.
    pub name: String,
    /// Whether the skill is up-to-date with the lockfile.
    pub fresh: bool,
    /// Human-readable reasons the skill is stale (empty when fresh).
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
    /// Whether every skill in the workspace is fresh.
    pub fresh: bool,
    /// Per-skill verdicts, sorted by name.
    pub skills: Vec<SkillResult>,
}

/// Computes the freshness report by hashing sources/outputs/fragments against
/// the lockfile. The returned [`CheckReport`] is sorted and has `fresh`
/// computed, ready to render.
pub fn verify(ws: &Workspace, lockfile: &Lockfile) -> CheckReport {
    let source_names: std::collections::HashSet<&str> =
        ws.skills.values().map(|s| s.name.as_str()).collect();

    let mut results: Vec<SkillResult> = ws
        .skills
        .values()
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

                    let skill_md = skill.target_dir.join("SKILL.md");
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
            for skill_name in &frag_entry.used_by {
                if let Some(result) = results.iter_mut().find(|s| &s.name == skill_name) {
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
    CheckReport {
        fresh: all_fresh,
        skills: results,
    }
}
