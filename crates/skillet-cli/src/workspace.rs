//! Workspace resolution: discovers all artifact types and collects them into
//! a single [`Workspace`] structure for the compile pipeline.
//!
//! Artifact types:
//! - **Skills** — `.pan` files under `{src_dir}/skills/{name}/{name}.pan`
//! - **Scripts** — files under `{src_dir}/skills/{skill}/scripts/`
//! - **References** — `.pan` files under `{src_dir}/skills/{skill}/references/**/*.pan`
//! - **Agents** — `.pan` files under `{src_dir}/agents/{name}/{name}.pan`

use anyhow::Result;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ── Artifact types ─────────────────────────────────────────────────────────────

/// A discovered skill within the workspace.
#[derive(Debug, Clone)]
pub struct Skill {
    /// The skill's directory name, used as its identifier.
    pub name: String,
    /// Absolute path to the `{name}.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's source directory.
    pub skill_dir: PathBuf,
    /// Scripts discovered within this skill.
    pub scripts: Vec<Script>,
    /// Reference `.pan` files discovered within this skill.
    pub references: Vec<Reference>,
}

/// A script file associated with a skill.
#[derive(Debug, Clone)]
pub struct Script {
    /// Path relative to the skill directory (e.g. `scripts/setup.sh`).
    pub relative_path: String,
    /// Absolute path to the script file.
    pub absolute_path: PathBuf,
}

/// A reference `.pan` file associated with a skill.
#[derive(Debug, Clone)]
pub struct Reference {
    /// Path relative to the skill directory (e.g. `references/api/types.pan`).
    pub relative_path: String,
    /// Absolute path to the reference file.
    pub absolute_path: PathBuf,
}

/// A discovered agent within the workspace.
#[derive(Debug, Clone)]
pub struct Agent {
    /// The agent's directory name, used as its identifier.
    pub name: String,
    /// Absolute path to the `{name}.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the agent's source directory.
    pub agent_dir: PathBuf,
}

/// All resolved workspace artifacts, ready to be passed to compile steps.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Root path of the workspace (where `skillet.toml` lives).
    pub root: PathBuf,
    /// All discovered skills with their associated scripts and references.
    pub skills: Vec<Skill>,
    /// All discovered agents.
    pub agents: Vec<Agent>,
}

// ── Resolution ─────────────────────────────────────────────────────────────────

/// Resolves the full workspace from the given root and source directory.
///
/// `src_dir` is the absolute path to the source directory (e.g. `workspace/src/skills`
/// parent — typically just `workspace/{skills_src_dir}/..` but we take the actual
/// configured `skills_src_dir` and `agents` sibling).
///
/// The layout expected:
/// ```text
/// {skills_src_dir}/
///   {skill_name}/
///     {skill_name}.pan
///     scripts/         (optional)
///     references/      (optional, *.pan files)
/// {agents_dir}/
///   {agent_name}/
///     {agent_name}.pan
/// ```
pub fn resolve(root: &Path, skills_src_dir: &Path, agents_dir: &Path) -> Result<Workspace> {
    let skills = resolve_skills(skills_src_dir)?;
    let agents = resolve_agents(agents_dir)?;

    Ok(Workspace {
        root: root.to_path_buf(),
        skills,
        agents,
    })
}

/// Discovers all skills under `skills_src_dir`, including their scripts and references.
fn resolve_skills(skills_src_dir: &Path) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();

    if !skills_src_dir.exists() {
        return Ok(skills);
    }

    for entry in WalkDir::new(skills_src_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let skill_dir = entry.path().to_path_buf();
        if !skill_dir.is_dir() {
            continue;
        }
        let dir_name = match skill_dir.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('_') && !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let source_path = skill_dir.join(format!("{dir_name}.pan"));
        if !source_path.exists() {
            continue;
        }

        let scripts = resolve_scripts(&skill_dir)?;
        let references = resolve_references(&skill_dir)?;

        skills.push(Skill {
            name: dir_name,
            source_path,
            skill_dir,
            scripts,
            references,
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Discovers script files under `{skill_dir}/scripts/`.
fn resolve_scripts(skill_dir: &Path) -> Result<Vec<Script>> {
    let scripts_dir = skill_dir.join("scripts");
    let mut scripts = Vec::new();

    if !scripts_dir.exists() {
        return Ok(scripts);
    }

    for entry in WalkDir::new(&scripts_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(skill_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        scripts.push(Script {
            relative_path: relative,
            absolute_path: path.to_path_buf(),
        });
    }

    scripts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(scripts)
}

/// Discovers `.pan` reference files under `{skill_dir}/references/`.
fn resolve_references(skill_dir: &Path) -> Result<Vec<Reference>> {
    let refs_dir = skill_dir.join("references");
    let mut references = Vec::new();

    if !refs_dir.exists() {
        return Ok(references);
    }

    for entry in WalkDir::new(&refs_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("pan") {
            continue;
        }
        let relative = path
            .strip_prefix(skill_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        references.push(Reference {
            relative_path: relative,
            absolute_path: path.to_path_buf(),
        });
    }

    references.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(references)
}

/// Discovers all agents under `agents_dir`.
fn resolve_agents(agents_dir: &Path) -> Result<Vec<Agent>> {
    let mut agents = Vec::new();

    if !agents_dir.exists() {
        return Ok(agents);
    }

    for entry in WalkDir::new(agents_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let agent_dir = entry.path().to_path_buf();
        if !agent_dir.is_dir() {
            continue;
        }
        let dir_name = match agent_dir.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('_') && !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let source_path = agent_dir.join(format!("{dir_name}.pan"));
        if !source_path.exists() {
            continue;
        }

        agents.push(Agent {
            name: dir_name,
            source_path,
            agent_dir,
        });
    }

    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolve_finds_skill_with_scripts_and_references() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let agents_dir = tmp.path().join("agents");

        // Create a skill with scripts and references
        let skill_dir = skills_dir.join("diagnose");
        fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        fs::create_dir_all(skill_dir.join("references/api")).unwrap();
        fs::write(skill_dir.join("diagnose.pan"), "---\nname: diagnose\n---\n").unwrap();
        fs::write(skill_dir.join("scripts/check.sh"), "#!/bin/bash\n").unwrap();
        fs::write(skill_dir.join("references/api/types.pan"), "# Types\n").unwrap();

        let ws = resolve(tmp.path(), &skills_dir, &agents_dir).unwrap();

        assert_eq!(ws.skills.len(), 1);
        assert_eq!(ws.skills[0].name, "diagnose");
        assert_eq!(ws.skills[0].scripts.len(), 1);
        assert_eq!(ws.skills[0].scripts[0].relative_path, "scripts/check.sh");
        assert_eq!(ws.skills[0].references.len(), 1);
        assert_eq!(
            ws.skills[0].references[0].relative_path,
            "references/api/types.pan"
        );
        assert!(ws.agents.is_empty());
    }

    #[test]
    fn resolve_finds_agents() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let agents_dir = tmp.path().join("agents");

        let agent_dir = agents_dir.join("reviewer");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("reviewer.pan"), "---\nname: reviewer\n---\n").unwrap();

        let ws = resolve(tmp.path(), &skills_dir, &agents_dir).unwrap();

        assert!(ws.skills.is_empty());
        assert_eq!(ws.agents.len(), 1);
        assert_eq!(ws.agents[0].name, "reviewer");
    }

    #[test]
    fn resolve_skips_underscore_and_dot_dirs() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let agents_dir = tmp.path().join("agents");

        // Underscore dir (fragments convention)
        let frags = skills_dir.join("_fragments");
        fs::create_dir_all(&frags).unwrap();
        fs::write(frags.join("_fragments.pan"), "").unwrap();

        // Dot dir
        let hidden = skills_dir.join(".hidden");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join(".hidden.pan"), "").unwrap();

        let ws = resolve(tmp.path(), &skills_dir, &agents_dir).unwrap();

        assert!(ws.skills.is_empty());
    }

    #[test]
    fn resolve_returns_empty_when_dirs_missing() {
        let tmp = TempDir::new().unwrap();
        let ws = resolve(
            tmp.path(),
            &tmp.path().join("nonexistent"),
            &tmp.path().join("also-missing"),
        )
        .unwrap();

        assert!(ws.skills.is_empty());
        assert!(ws.agents.is_empty());
    }

    #[test]
    fn resolve_ignores_skill_dir_without_pan_file() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let agents_dir = tmp.path().join("agents");

        // Directory exists but has no matching .pan file
        let skill_dir = skills_dir.join("broken");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("README.md"), "# Not a skill\n").unwrap();

        let ws = resolve(tmp.path(), &skills_dir, &agents_dir).unwrap();

        assert!(ws.skills.is_empty());
    }

    #[test]
    fn references_only_collects_pan_files() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let agents_dir = tmp.path().join("agents");

        let skill_dir = skills_dir.join("myskill");
        fs::create_dir_all(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("myskill.pan"), "---\nname: myskill\n---\n").unwrap();
        fs::write(skill_dir.join("references/good.pan"), "# ref\n").unwrap();
        fs::write(skill_dir.join("references/ignored.md"), "# not pan\n").unwrap();

        let ws = resolve(tmp.path(), &skills_dir, &agents_dir).unwrap();

        assert_eq!(ws.skills[0].references.len(), 1);
        assert_eq!(
            ws.skills[0].references[0].relative_path,
            "references/good.pan"
        );
    }

    #[test]
    fn skills_are_sorted_by_name() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let agents_dir = tmp.path().join("agents");

        for name in ["zulu", "alpha", "mike"] {
            let dir = skills_dir.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(format!("{name}.pan")),
                format!("---\nname: {name}\n---\n"),
            )
            .unwrap();
        }

        let ws = resolve(tmp.path(), &skills_dir, &agents_dir).unwrap();

        let names: Vec<&str> = ws.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mike", "zulu"]);
    }
}
