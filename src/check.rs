//! Freshness verification: compares workspace state against `skillet.lock`.
//!
//! `skillet check` is designed as a fast CI gate — it never recompiles; it
//! just hashes source files and on-disk `SKILL.md` files and compares them
//! against the recorded hashes in the lockfile.

use crate::config;
use crate::workspace;
use anyhow::{Context, Result};
use serde::Serialize;
use sha2::Digest;
use std::path::Path;

/// How results are rendered.
#[derive(Clone, Copy, Default)]
pub enum OutputFormat {
    /// Human-readable output (default).
    #[default]
    Human,
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
    /// Human-readable reasons this skill is considered stale (empty when fresh).
    pub reasons: Vec<String>,
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
pub fn run(workspace: &Path, format: OutputFormat) -> Result<bool> {
    // ── load config & discover sources ─────────────────────────────────────
    let config = config::load(workspace)?;
    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;

    // ── require a lockfile ─────────────────────────────────────────────────
    let lock_path = workspace.join("skillet.lock");
    if !lock_path.exists() {
        anyhow::bail!(
            "skillet.lock not found — run `skillet build` to generate it"
        );
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

            match lockfile.skills.get(&source.name) {
                None => {
                    reasons.push(format!(
                        "skill '{}' not in lockfile — run `skillet build`",
                        source.name
                    ));
                }
                Some(entry) => {
                    // Fast path: compare source hash.
                    match hash_file(&source.source_path) {
                        Err(e) => reasons.push(format!("could not hash source: {}", e)),
                        Ok(current_source_hash) => {
                            if current_source_hash != entry.source_hash {
                                reasons.push(format!(
                                    "source '{}' has changed since last build",
                                    source
                                        .source_path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                ));
                            }
                        }
                    }

                    // Check on-disk SKILL.md against the recorded compiled hash.
                    let skill_md = source.skill_out_dir.join("SKILL.md");
                    if !skill_md.exists() {
                        reasons.push("SKILL.md is missing — run `skillet build`".to_string());
                    } else {
                        match hash_file(&skill_md) {
                            Err(e) => reasons.push(format!("could not hash SKILL.md: {}", e)),
                            Ok(current_compiled_hash) => {
                                if current_compiled_hash != entry.compiled_hash {
                                    reasons.push(
                                        "SKILL.md does not match last build — run `skillet build`"
                                            .to_string(),
                                    );
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
            }
        })
        .collect();

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
        OutputFormat::Human => render_human(&report),
    }

    Ok(all_fresh)
}

// ── rendering ────────────────────────────────────────────────────────────────

fn render_human(report: &CheckReport) {
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

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {} for hashing", path.display()))?;
    Ok(format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(&bytes))
    ))
}

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
        crate::build::run(tmp, None).unwrap();
    }

    #[test]
    fn check_passes_on_freshly_built_workspace() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        setup_built_workspace(tmp.path());

        // Act
        let ok = run(tmp.path(), OutputFormat::Human).unwrap();

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
        let ok = run(tmp.path(), OutputFormat::Human).unwrap();

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
        let ok = run(tmp.path(), OutputFormat::Human).unwrap();

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
        let ok = run(tmp.path(), OutputFormat::Human).unwrap();

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
        let result = run(tmp.path(), OutputFormat::Human);

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
        let ok = run(tmp.path(), OutputFormat::Json).unwrap();

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
        let ok = run(tmp.path(), OutputFormat::Human).unwrap();

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
        let ok = run(tmp.path(), OutputFormat::Human).unwrap();

        // Assert — new skill not in lockfile → stale
        assert!(!ok);
    }
}
