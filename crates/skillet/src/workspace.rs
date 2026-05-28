//! Workspace resolution: discovers all artifact types and provides a single
//! [`ResolvedWorkspace`] structure shared by the build and lint pipelines.
//!
//! Artifact types:
//! - **Skills** — `.pan` files under `{skills_src_dir}/{name}/{name}.pan`
//! - **Scripts** — files under `{skills_src_dir}/{skill}/scripts/`
//! - **References** — `.pan` files under `{skills_src_dir}/{skill}/references/**/*.pan`
//! - **Agents** — `.pan` files under `agents/{name}/{name}.pan`
//! - **Fragments** — `.fragment.pan` files under `{fragments_dir}/`

use anyhow::{Context, Result};
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

use crate::compiler::{render_fragments, RenderedFragments};
use crate::config::SkilletConfig;

static CMD_REF_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"`cmd::([^`]+)`").expect("valid cmd ref regex"));

// ── Artifact types ─────────────────────────────────────────────────────────────

/// A discovered skill within the workspace.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Skill {
    /// The skill's directory name, used as its identifier.
    pub name: String,
    /// Absolute path to the `{name}.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's source directory.
    pub skill_dir: PathBuf,
    /// Absolute path to the skill's output directory.
    pub skill_out_dir: PathBuf,
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
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Agent {
    /// The agent's directory name, used as its identifier.
    pub name: String,
    /// Absolute path to the `{name}.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the agent's source directory.
    pub agent_dir: PathBuf,
}

// ── ResolvedWorkspace ──────────────────────────────────────────────────────────

/// Fully-resolved workspace: all artifacts discovered, fragments rendered,
/// commands checked against PATH.  Constructed once and shared by build/lint.
#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    /// Workspace root (where `skillet.toml` lives).
    pub root: PathBuf,
    /// All discovered skills (sorted by name).
    pub skills: Vec<Skill>,
    /// All discovered agents (sorted by name).
    pub agents: Vec<Agent>,
    /// Raw fragment content keyed by fragment name.
    pub raw_fragments: HashMap<String, String>,
    /// Pre-rendered fragments (ready for interpolation).
    pub rendered_fragments: RenderedFragments,
    /// Commands confirmed present on PATH.  Contains a sentinel `"\x01"` when
    /// any `cmd::` refs exist, ensuring compile_body's empty-set guard doesn't
    /// suppress validation.
    pub known_commands: HashSet<String>,
    /// SHA-256 hash per fragment (`"sha256:<hex>"`).
    pub fragment_hashes: HashMap<String, String>,
    /// Token count per fragment.
    pub fragment_tokens: HashMap<String, u32>,
}

impl ResolvedWorkspace {
    /// Resolves the full workspace from the given root directory and config.
    ///
    /// Performs all filesystem I/O: discovers skills, agents, fragments; renders
    /// fragments; scans source files for `cmd::` refs and probes PATH.
    pub fn resolve(root: &Path, cfg: &SkilletConfig) -> Result<Self> {
        let skills_src_dir = root.join(&cfg.workspace.skills_src_dir);
        let skills_out_dir = root.join(&cfg.workspace.skills_out_dir);
        let fragments_dir = root.join(&cfg.workspace.fragments_dir);
        let agents_dir = root.join("agents");

        let skills = discover_skills(&skills_src_dir, &skills_out_dir)?;
        let agents = discover_agents(&agents_dir)?;

        let raw_fragments = load_all_fragments(&fragments_dir)?;
        let rendered_fragments = render_fragments(&raw_fragments);

        let mut fragment_hashes = HashMap::new();
        let mut fragment_tokens = HashMap::new();
        for (name, content) in &raw_fragments {
            fragment_hashes.insert(name.clone(), hash_bytes(content.as_bytes()));
            fragment_tokens.insert(
                name.clone(),
                crate::tokens::count_tokens(content, &cfg.build.tokenizer),
            );
        }

        let known_commands = collect_known_commands(&skills);

        Ok(Self {
            root: root.to_path_buf(),
            skills,
            agents,
            raw_fragments,
            rendered_fragments,
            known_commands,
            fragment_hashes,
            fragment_tokens,
        })
    }

    /// Set of skill names in the workspace.
    pub fn skill_names(&self) -> HashSet<&str> {
        self.skills.iter().map(|s| s.name.as_str()).collect()
    }

    /// Set of agent names in the workspace.
    pub fn agent_names(&self) -> HashSet<&str> {
        self.agents.iter().map(|a| a.name.as_str()).collect()
    }

    /// Fragment names present in the workspace.
    pub fn fragment_names(&self) -> Vec<&str> {
        self.raw_fragments.keys().map(|k| k.as_str()).collect()
    }

    /// Returns relative file paths within a skill's directory.
    pub fn skill_files(&self, skill: &Skill) -> HashSet<String> {
        let mut files = HashSet::new();
        if !skill.skill_dir.exists() {
            return files;
        }
        for entry in WalkDir::new(&skill.skill_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().is_file() {
                if let Ok(rel) = entry.path().strip_prefix(&skill.skill_dir) {
                    files.insert(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        files
    }
}

// ── Discovery functions ────────────────────────────────────────────────────────

fn discover_skills(skills_src_dir: &Path, skills_out_dir: &Path) -> Result<Vec<Skill>> {
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
        let source_path = skill_dir.join(format!("{}.pan", dir_name));
        if !source_path.exists() {
            continue;
        }

        let scripts = resolve_scripts(&skill_dir)?;
        let references = resolve_references(&skill_dir)?;

        skills.push(Skill {
            name: dir_name.clone(),
            source_path,
            skill_dir,
            skill_out_dir: skills_out_dir.join(&dir_name),
            scripts,
            references,
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

fn discover_agents(agents_dir: &Path) -> Result<Vec<Agent>> {
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
        let source_path = agent_dir.join(format!("{}.pan", dir_name));
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

/// Pre-scans all skill sources for `cmd::` refs and checks each against PATH.
fn collect_known_commands(skills: &[Skill]) -> HashSet<String> {
    let mut known = HashSet::new();
    let mut found_any = false;

    for skill in skills {
        let content = match std::fs::read_to_string(&skill.source_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for caps in CMD_REF_RE.captures_iter(&content) {
            found_any = true;
            let full_cmd = caps[1].trim();
            let cmd = full_cmd.split_whitespace().next().unwrap_or(full_cmd);
            if !known.contains(cmd) && is_on_path(cmd) {
                known.insert(cmd.to_string());
            }
        }
    }

    if found_any {
        known.insert("\x01".to_string());
    }
    known
}

// ── Utility functions ───────────────────────────────────────────────────────────

/// Loads a fragment by name from `fragments_dir`.
///
/// Expects a file named `{name}.fragment.pan`.
pub fn load_fragment(fragments_dir: &Path, name: &str) -> Result<String> {
    let path = fragments_dir.join(format!("{}.fragment.pan", name));
    std::fs::read_to_string(&path).with_context(|| {
        format!(
            "fragment '{}' not found (expected at {})",
            name,
            path.display()
        )
    })
}

/// Recursively copies `src` into `dest`, preserving the directory tree.
pub fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel == std::path::Path::new("") {
            continue;
        }
        let target = dest.join(rel);
        if path.is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("failed to create directory {}", target.display()))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            std::fs::copy(path, &target).with_context(|| {
                format!("failed to copy {} to {}", path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

/// Returns `"sha256:<hex>"` of the file at `path`.
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {} for hashing", path.display()))?;
    Ok(format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(&bytes))
    ))
}

/// Returns `"sha256:<hex>"` of `bytes` (in-memory hashing).
pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
}

/// Returns `true` if `cmd` is found as a file in any directory on `PATH`.
pub fn is_on_path(cmd: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(cmd).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolve_finds_skill_with_scripts_and_references() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();

        let skills_dir = tmp.path().join("src/skills");
        let skill_dir = skills_dir.join("diagnose");
        fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        fs::create_dir_all(skill_dir.join("references/api")).unwrap();
        fs::write(skill_dir.join("diagnose.pan"), "---\nname: diagnose\n---\n").unwrap();
        fs::write(skill_dir.join("scripts/check.sh"), "#!/bin/bash\n").unwrap();
        fs::write(skill_dir.join("references/api/types.pan"), "# Types\n").unwrap();

        let ws = ResolvedWorkspace::resolve(tmp.path(), &cfg).unwrap();

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
        let cfg = SkilletConfig::default();

        let agent_dir = tmp.path().join("agents/reviewer");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("reviewer.pan"), "---\nname: reviewer\n---\n").unwrap();

        // Also create the skills src dir so resolution doesn't fail
        fs::create_dir_all(tmp.path().join("src/skills")).unwrap();

        let ws = ResolvedWorkspace::resolve(tmp.path(), &cfg).unwrap();

        assert!(ws.skills.is_empty());
        assert_eq!(ws.agents.len(), 1);
        assert_eq!(ws.agents[0].name, "reviewer");
    }

    #[test]
    fn resolve_skips_underscore_and_dot_dirs() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();

        let skills_dir = tmp.path().join("src/skills");
        let frags = skills_dir.join("_fragments");
        fs::create_dir_all(&frags).unwrap();
        fs::write(frags.join("_fragments.pan"), "").unwrap();

        let hidden = skills_dir.join(".hidden");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join(".hidden.pan"), "").unwrap();

        let ws = ResolvedWorkspace::resolve(tmp.path(), &cfg).unwrap();
        assert!(ws.skills.is_empty());
    }

    #[test]
    fn resolve_returns_empty_when_dirs_missing() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();
        let ws = ResolvedWorkspace::resolve(tmp.path(), &cfg).unwrap();
        assert!(ws.skills.is_empty());
        assert!(ws.agents.is_empty());
    }

    #[test]
    fn resolve_loads_and_renders_fragments() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();

        let frags_dir = tmp.path().join("src/skills/_fragments");
        fs::create_dir_all(&frags_dir).unwrap();
        fs::write(frags_dir.join("check-adrs.fragment.pan"), "## Check ADRs\n").unwrap();

        let ws = ResolvedWorkspace::resolve(tmp.path(), &cfg).unwrap();

        assert_eq!(ws.raw_fragments.len(), 1);
        assert!(ws.raw_fragments.contains_key("check-adrs"));
        assert!(ws.rendered_fragments.rendered.contains_key("check-adrs"));
        assert!(ws.fragment_hashes.contains_key("check-adrs"));
        assert!(ws.fragment_tokens.contains_key("check-adrs"));
    }

    #[test]
    fn skill_names_returns_set() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();

        let skills_dir = tmp.path().join("src/skills");
        for name in ["alpha", "zulu"] {
            let dir = skills_dir.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(format!("{name}.pan")),
                format!("---\nname: {name}\n---\n"),
            )
            .unwrap();
        }

        let ws = ResolvedWorkspace::resolve(tmp.path(), &cfg).unwrap();
        let names = ws.skill_names();
        assert!(names.contains("alpha"));
        assert!(names.contains("zulu"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn load_fragment_reads_dot_fragment_pan_file() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("check-adrs.fragment.pan"),
            "## Check ADRs\n",
        )
        .unwrap();

        let content = load_fragment(tmp.path(), "check-adrs").unwrap();
        assert_eq!(content, "## Check ADRs\n");
    }

    #[test]
    fn load_fragment_errors_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let result = load_fragment(tmp.path(), "missing");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }

    #[test]
    fn copy_dir_recursive_preserves_structure_and_content() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub/b.txt"), "world").unwrap();
        let dest = tmp.path().join("dest");

        copy_dir_recursive(&src, &dest).unwrap();

        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dest.join("sub/b.txt")).unwrap(), "world");
    }
}
