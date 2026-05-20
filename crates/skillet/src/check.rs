//! Freshness verification: compares workspace state against `skillet.lock`.
//!
//! `skillet check` is designed as a fast CI gate — it never recompiles; it
//! just hashes source files and on-disk `SKILL.md` files and compares them
//! against the recorded hashes in the lockfile.

use crate::config::SkilletConfig;
use crate::workspace;
use anyhow::{Context, Result};
use serde::Serialize;
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
    /// Skill name.
    pub name: String,
    /// `true` when source hash **and** compiled hash both match the lockfile.
    pub fresh: bool,
    /// Reasons this skill is considered stale (empty when fresh).
    pub reasons: Vec<String>,
    /// Machine-readable difference entries (populated in JSON mode; empty when fresh).
    pub diffs: Vec<DiffEntry>,
}

/// A single machine-readable staleness difference.
#[derive(Debug, Serialize)]
pub struct DiffEntry {
    /// Kind of difference: `"source_changed"`, `"skill_md_missing"`,
    /// `"skill_md_changed"`, `"fragment_changed"`, `"fragment_missing"`,
    /// `"not_in_lockfile"`, `"source_dir_missing"`.
    pub kind: String,
    /// File or entity involved, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Overall `check` output.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    /// `true` when every skill is fresh.
    pub fresh: bool,
    /// Per-skill results (alphabetical order).
    pub skills: Vec<SkillResult>,
}

/// Runs freshness checks for all skills in the workspace.
///
/// Returns `Ok(true)` when everything is up-to-date, `Ok(false)` when one or
/// more skills are stale.
///
/// # Errors
///
/// Returns an error if `skillet.lock` is absent, unreadable, or if the
/// workspace configuration cannot be loaded.
pub fn run(workspace: &Path, format: OutputFormat, config: &SkilletConfig) -> Result<bool> {
    // ── load config & discover sources ─────────────────────────────────────
    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);
    let sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;

    // ── require a lockfile ─────────────────────────────────────────────────
    let lock_path = workspace.join("skillet.lock");
    if !lock_path.exists() {
        anyhow::bail!("skillet.lock not found — run `skillet build` to generate it");
    }
    let lockfile = crate::lockfile::read(workspace)
        .with_context(|| format!("failed to read {}", lock_path.display()))?;

    // ── check each discovered skill ────────────────────────────────────────
    let source_names: std::collections::HashSet<&str> =
        sources.iter().map(|s| s.name.as_str()).collect();

    let mut results: Vec<SkillResult> = sources
        .iter()
        .map(|source| {
            let mut reasons: Vec<String> = Vec::new();
            let mut diffs: Vec<DiffEntry> = Vec::new();

            match lockfile.skills.get(&source.name) {
                None => {
                    reasons.push(format!(
                        "skill '{}' not in lockfile — run `skillet build`",
                        source.name
                    ));
                    diffs.push(DiffEntry {
                        kind: "not_in_lockfile".to_string(),
                        file: None,
                    });
                }
                Some(entry) => {
                    // Fast path: compare source hash.
                    match workspace::hash_file(&source.source_path) {
                        Err(e) => reasons.push(format!("could not hash source: {}", e)),
                        Ok(current_source_hash) => {
                            if current_source_hash != entry.source_hash {
                                let fname = source
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
                                    file: Some(source.source_path.to_string_lossy().to_string()),
                                });
                            }
                        }
                    }

                    // Check on-disk SKILL.md against the recorded compiled hash.
                    let skill_md = source.skill_out_dir.join("SKILL.md");
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
                name: source.name.clone(),
                fresh,
                reasons,
                diffs,
            }
        })
        .collect();

    // ── verify fragment hashes against lockfile ──────────────────────────────
    // If a fragment file has changed since the last build, mark all skills that
    // include it as stale.  This runs even when there are no skills in `results`.
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

    // ── flag lockfile entries whose source directory no longer exists ─────────
    // A skill removed from disk without rebuilding would otherwise appear fresh.
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

    // Stable alphabetical order regardless of discovery order.
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

// ── rendering ────────────────────────────────────────────────────────────────

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

// ── hashing ──────────────────────────────────────────────────────────────────

// File hashing delegated to workspace::hash_file.

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;
    use std::fs;
    use tempfile::TempDir;

    /// Minimal workspace with one built skill ready for freshness checks.
    fn setup_built_workspace(tmp: &Path) {
        let cfg = SkilletConfig::default();
        fs::write(tmp.join("skillet.toml"), cfg.to_toml().unwrap()).unwrap();
        fs::create_dir_all(tmp.join("src/skills/_fragments")).unwrap();
        fs::create_dir_all(tmp.join("skills")).unwrap();

        let skill_src_dir = tmp.join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n",
        )
        .unwrap();

        // Run build so the lockfile and SKILL.md are consistent.
        crate::compile::run(tmp, None, &Default::default(), &cfg).unwrap();
    }

    #[test]
    fn check_passes_on_freshly_built_workspace() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert
        assert!(ok);
    }

    #[test]
    fn check_fails_when_source_edited_after_build() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Modify the source after build
        let skill_src = tmp.path().join("src/skills/my-skill/my-skill.pan");
        fs::write(
            &skill_src,
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill (edited)\n",
        )
        .unwrap();

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert
        assert!(!ok);
    }

    #[test]
    fn check_fails_when_skill_md_manually_edited() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Tamper with the compiled output
        let skill_md = tmp.path().join("skills/my-skill/SKILL.md");
        let original = fs::read_to_string(&skill_md).unwrap();
        fs::write(&skill_md, format!("{}\n<!-- tampered -->", original)).unwrap();

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert
        assert!(!ok);
    }

    #[test]
    fn check_fails_when_skill_md_missing() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Remove the compiled output
        fs::remove_file(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert
        assert!(!ok);
    }

    #[test]
    fn check_errors_when_lockfile_absent() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();
        fs::write(tmp.path().join("skillet.toml"), cfg.to_toml().unwrap()).unwrap();
        fs::create_dir_all(tmp.path().join("skills")).unwrap();

        // Act — no skillet.lock present
        let result = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default());

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("skillet build"));
    }

    #[test]
    fn check_json_format_produces_valid_json() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Act — just ensure it doesn't error and returns Ok(true)
        let ok = run(tmp.path(), OutputFormat::Json, &SkilletConfig::default()).unwrap();

        // Assert
        assert!(ok);
    }

    #[test]
    fn check_fails_when_skill_deleted_after_build() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Remove the skill directory without rebuilding
        fs::remove_dir_all(tmp.path().join("skills/my-skill")).unwrap();

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert — lockfile still references my-skill → stale
        assert!(!ok);
    }

    #[test]
    fn check_fails_when_new_skill_added_after_build() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Add a second skill without rebuilding
        let new_dir = tmp.path().join("src/skills/new-skill");
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(
            new_dir.join("new-skill.pan"),
            "---\nname: new-skill\ndescription: \"\"\n---\n\n# New\n",
        )
        .unwrap();

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert — new skill not in lockfile → stale
        assert!(!ok);
    }

    #[test]
    fn check_fails_when_fragment_edited_after_build() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();
        fs::write(tmp.path().join("skillet.toml"), cfg.to_toml().unwrap()).unwrap();
        fs::create_dir_all(tmp.path().join("src/skills/_fragments")).unwrap();
        fs::create_dir_all(tmp.path().join("skills")).unwrap();

        // Create a fragment and a skill that uses it
        fs::write(
            tmp.path().join("src/skills/_fragments/note.fragment.pan"),
            "## Note\noriginal content\n",
        )
        .unwrap();
        let skill_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: \"test\"\n---\n\n{{> note }}\n",
        )
        .unwrap();
        crate::compile::run(
            tmp.path(),
            None,
            &Default::default(),
            &SkilletConfig::default(),
        )
        .unwrap();

        // Edit the fragment without rebuilding
        fs::write(
            tmp.path().join("src/skills/_fragments/note.fragment.pan"),
            "## Note\nmodified content\n",
        )
        .unwrap();

        // Act
        let ok = run(tmp.path(), OutputFormat::Text, &SkilletConfig::default()).unwrap();

        // Assert — fragment changed → stale
        assert!(!ok, "check should report stale when fragment has changed");
    }
}
