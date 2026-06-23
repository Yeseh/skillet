//! Workspace resolution: discovers all artifact types and provides a single
//! [`Workspace`] structure shared by the build and lint pipelines.
//!
//! Artifact types:
//! - **Skills** — `.pan` files under `{module.src_dir}/{name}/{name}.pan`
//! - **Scripts** — files under `{module.src_dir}/{skill}/scripts/`
//! - **References** — `.pan` files under `{module.src_dir}/{skill}/references/**/*.pan`
//! - **Agents** — `.pan` files under `agents/{name}/{name}.pan`
//! - **Fragments** — `.fragment.pan` files under `workspace.fragments_dir` (global)
//! or `module.fragments_dir` (module-local)

/// Skill, Script, and Reference type definitions.
pub mod skill;

pub use skill::Skill;

use anyhow::{Context, Result};
use sha2::Digest;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::compiler::compile::{render_fragments, RenderedFragments};
use crate::config::{EnvVar, SkilletConfig};
use crate::workspace::skill::{Reference, Script};

// ── Artifact types ─────────────────────────────────────────────────────────────

/// A discovered agent within the workspace.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Agent {
    /// The agent's name, used as its identifier.
    pub name: String,
    /// Absolute path to the `{name}.pan` source file.
    pub source_path: PathBuf,
    /// The target path of the agent after compilation
    pub target_path: PathBuf,
}

/// Fully-resolved workspace: all artifacts discovered, fragments rendered,
/// commands checked against PATH.  Constructed once and shared by build/lint.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Workspace root (where `skillet.toml` lives).
    pub root: PathBuf,
    /// All discovered skills (sorted by name).
    pub skills: HashMap<String, Skill>,
    /// All discovered agents (sorted by name).
    pub agents: HashMap<String, Agent>,
    /// Raw fragment content keyed by fragment name.
    pub raw_fragments: HashMap<String, String>,
    /// Pre-rendered fragments (ready for interpolation).
    pub fragments: RenderedFragments,
    /// SHA-256 hash per fragment (`"sha256:<hex>"`).
    pub fragment_hashes: HashMap<String, String>,
    /// Token count per fragment.
    pub fragment_tokens: HashMap<String, u32>,
    /// Absolute path to each fragment's source file, keyed by fragment name.
    pub fragment_paths: HashMap<String, PathBuf>,
    /// Fragment names that come from the workspace-global `fragments_dir`.
    pub global_fragment_names: HashSet<String>,
    /// Fragment names local to each module, keyed by module name.
    pub module_fragment_names: HashMap<String, HashSet<String>>,
    /// Variable substitutions from `[vars]` in `skillet.toml`.
    pub vars: BTreeMap<String, String>,
    /// Declared environment variables with defaults from `[env]`.
    pub env: BTreeMap<String, EnvVar>,
    /// Commands treated as available regardless of PATH (`allowed_commands`).
    pub allowed_commands: HashSet<String>,
    /// Tiktoken encoding name used for all token counting.
    pub tokenizer: String,
}

impl Workspace {
    /// Resolves the full workspace from the given root directory and config.
    /// Performs all filesystem I/O: discovers skills from every module, agents,
    /// and fragments (global + per-module); renders fragments; scans source
    /// files for `cmd::` refs and probes PATH.
    pub fn resolve(root: &Path, cfg: &SkilletConfig) -> Result<Self> {
        let agents_dir = root.join("agents");

        // ── Step 1: workspace-global fragments ─────────────────────────────
        let mut raw_fragments: HashMap<String, String> = HashMap::new();
        let mut fragment_paths: HashMap<String, PathBuf> = HashMap::new();
        let mut global_fragment_names: HashSet<String> = HashSet::new();

        if let Some(frags_rel) = &cfg.workspace.fragments_dir {
            let frags_dir = root.join(frags_rel);
            let global_frags = load_all_fragments_with_paths(&frags_dir)?;
            for (name, (content, path)) in global_frags {
                global_fragment_names.insert(name.clone());
                fragment_paths.insert(name.clone(), path);
                raw_fragments.insert(name, content);
            }
        }

        // ── Step 2: per-module skills and local fragments ───────────────────
        let mut all_skills: Vec<Skill> = Vec::new();
        let mut module_fragment_names: HashMap<String, HashSet<String>> = HashMap::new();

        for (module_name, module_cfg) in &cfg.modules {
            // Module-local fragments (override global for same name)
            if let Some(frags_rel) = &module_cfg.fragments_dir {
                let frags_dir = root.join(frags_rel);
                let mod_frags = load_all_fragments_with_paths(&frags_dir)?;
                let mut local_names = HashSet::new();
                for (name, (content, path)) in mod_frags {
                    local_names.insert(name.clone());
                    fragment_paths.insert(name.clone(), path);
                    raw_fragments.insert(name, content);
                }
                module_fragment_names.insert(module_name.clone(), local_names);
            }

            let src_dir = root.join(&module_cfg.src_dir);
            let out_dir = root.join(&module_cfg.out_dir);
            let mut skills = discover_skills(&src_dir, &out_dir)?;
            for skill in &mut skills {
                skill.module = module_name.clone();
            }
            all_skills.extend(skills);
        }

        // ── Step 3: agents (workspace-level, use first module's out_dir) ───
        let first_out_dir = cfg
            .modules
            .values()
            .next()
            .map(|m| root.join(&m.out_dir))
            .unwrap_or_else(|| root.join("skills"));

        let agents: HashMap<String, Agent> = discover_agents(&agents_dir, &first_out_dir)?
            .into_iter()
            .map(|a| (a.name.clone(), a))
            .collect();

        // ── Step 4: render, hash, count fragments ───────────────────────────
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

        let skills: HashMap<String, Skill> = all_skills
            .into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();

        Ok(Self {
            root: root.to_path_buf(),
            skills,
            agents,
            raw_fragments,
            fragments: rendered_fragments,
            fragment_hashes,
            fragment_tokens,
            fragment_paths,
            global_fragment_names,
            module_fragment_names,
            vars: cfg.vars.clone(),
            env: cfg.env.clone(),
            allowed_commands: cfg.allowed_commands.clone(),
            tokenizer: cfg.build.tokenizer.clone(),
        })
    }

    /// Set of skill names in the workspace.
    pub fn skill_names(&self) -> HashSet<&str> {
        self.skills.keys().map(|k| k.as_str()).collect()
    }

    /// Set of agent names in the workspace.
    pub fn agent_names(&self) -> HashSet<&str> {
        self.agents.keys().map(|k| k.as_str()).collect()
    }

    /// Fragment names present in the workspace.
    pub fn fragment_names(&self) -> Vec<&str> {
        self.raw_fragments.keys().map(|k| k.as_str()).collect()
    }

    /// Returns relative paths of all script files within a skill's `scripts/` directory.
    pub fn get_scripts_for_skill(&self, skill: &Skill) -> HashSet<String> {
        self.child_files_for_skill(skill, Some(Path::new("scripts")))
    }

    /// Returns relative paths of all reference files within a skill's `references/` directory.
    pub fn get_references_for_skill(&self, skill: &Skill) -> HashSet<String> {
        self.child_files_for_skill(skill, Some(Path::new("references")))
    }

    /// Returns relative paths (slash-normalised) of every file under the skill's source dir.
    pub fn get_source_files_for_skill(&self, skill: &Skill) -> HashSet<String> {
        self.child_files_for_skill(skill, None)
    }

    /// Returns relative file paths within a skill's directory.
    fn child_files_for_skill(&self, skill: &Skill, sub_folder: Option<&Path>) -> HashSet<String> {
        let mut files = HashSet::new();
        if !skill.src_dir.exists() {
            return files;
        }

        let target_dir = match sub_folder {
            Some(subdir) => &skill.src_dir.join(subdir),
            None => &skill.src_dir,
        };

        for entry in WalkDir::new(target_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().is_file() {
                if let Ok(rel) = entry.path().strip_prefix(&skill.src_dir) {
                    files.insert(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }

        files
    }
}

// ── Discovery functions ────────────────────────────────────────────────────────

fn discover_skills(src_dir: &Path, out_dir: &Path) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();

    if !src_dir.exists() {
        return Ok(skills);
    }

    for entry in WalkDir::new(src_dir)
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
            module: String::new(), // stamped by caller after discovery
            source_path,
            src_dir: skill_dir,
            target_dir: out_dir.join(&dir_name),
            scripts,
            references,
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

fn discover_agents(source_dir: &Path, out_dir: &Path) -> Result<Vec<Agent>> {
    if !source_dir.exists() {
        return Ok(Vec::new());
    }

    let mut agents: Vec<Agent> = WalkDir::new(source_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.into_path();
            let ext = path.extension()?.to_str()?;
            if !path.is_file() || (ext != "pan" && ext != "md") {
                return None;
            }
            let filename = path.file_stem()?.to_str()?;
            if filename.starts_with('_') || filename.starts_with('.') {
                return None;
            }

            let target_dir = out_dir.join(PathBuf::from("agents"));
            Some(Agent {
                name: filename.to_string(),
                source_path: path.clone(),
                target_path: target_dir.join(format!("{}.md", filename)),
            })
        })
        .collect();

    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

fn resolve_scripts(skill_dir: &Path) -> Result<Vec<Script>> {
    let scripts_dir = skill_dir.join("scripts");

    if !scripts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut scripts: Vec<Script> = WalkDir::new(&scripts_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.into_path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let relative = path
                .strip_prefix(skill_dir)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some(Script {
                name,
                relative_path: relative,
                absolute_path: path,
            })
        })
        .collect();

    scripts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(scripts)
}

fn resolve_references(skill_dir: &Path) -> Result<Vec<Reference>> {
    let refs_dir = skill_dir.join("references");

    if !refs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut references: Vec<Reference> = WalkDir::new(&refs_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.into_path();
            if !path.is_file() || path.extension()?.to_str() != Some("pan") {
                return None;
            }
            let rel_to_refs = path.strip_prefix(&refs_dir).ok()?;
            let name = rel_to_refs
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            let relative = path
                .strip_prefix(skill_dir)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some(Reference {
                name,
                relative_path: relative,
                absolute_path: path,
            })
        })
        .collect();

    references.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(references)
}

/// Loads all fragments from `fragments_dir`, returning `(name, (content, path))` pairs.
fn load_all_fragments_with_paths(
    fragments_dir: &Path,
) -> Result<HashMap<String, (String, PathBuf)>> {
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
        let path = entry.path().to_path_buf();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".fragment.pan"))
        {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read fragment '{}'", path.display()))?;
            map.insert(name.to_string(), (content, path));
        }
    }
    Ok(map)
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

        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();

        assert_eq!(ws.skills.len(), 1);
        let skill = ws.skills.get("diagnose").expect("skill 'diagnose'");
        assert_eq!(skill.name, "diagnose");
        assert_eq!(skill.module, "default");
        assert_eq!(skill.scripts.len(), 1);
        assert_eq!(skill.scripts[0].relative_path, "scripts/check.sh");
        assert_eq!(skill.references.len(), 1);
        assert_eq!(
            skill.references[0].relative_path,
            "references/api/types.pan"
        );
        assert_eq!(skill.references[0].name, "api/types");
        assert!(ws.agents.is_empty());
    }

    #[test]
    fn resolve_finds_agents() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();

        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("reviewer.pan"),
            "---\nname: reviewer\n---\n",
        )
        .unwrap();

        // Also create the skills src dir so resolution doesn't fail
        fs::create_dir_all(tmp.path().join("src/skills")).unwrap();

        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();

        assert!(ws.skills.is_empty());
        assert_eq!(ws.agents.len(), 1);
        assert_eq!(
            ws.agents.get("reviewer").expect("agent 'reviewer'").name,
            "reviewer"
        );
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

        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();
        assert!(ws.skills.is_empty());
    }

    #[test]
    fn resolve_returns_empty_when_dirs_missing() {
        let tmp = TempDir::new().unwrap();
        let cfg = SkilletConfig::default();
        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();
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

        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();

        assert_eq!(ws.raw_fragments.len(), 1);
        assert!(ws.raw_fragments.contains_key("check-adrs"));
        assert!(ws.fragments.rendered.contains_key("check-adrs"));
        assert!(ws.fragment_hashes.contains_key("check-adrs"));
        assert!(ws.fragment_tokens.contains_key("check-adrs"));
        assert!(ws.fragment_paths.contains_key("check-adrs"));
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

        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();
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
    fn global_fragments_tracked_separately_from_module_local() {
        let tmp = TempDir::new().unwrap();

        // Config with a global fragment dir and a module with its own
        let mut cfg = SkilletConfig::default();
        cfg.workspace.fragments_dir = Some("global-frags".to_string());
        if let Some(m) = cfg.modules.get_mut("default") {
            m.fragments_dir = Some("local-frags".to_string());
        }

        let global_dir = tmp.path().join("global-frags");
        let local_dir = tmp.path().join("local-frags");
        fs::create_dir_all(&global_dir).unwrap();
        fs::create_dir_all(&local_dir).unwrap();
        fs::write(global_dir.join("shared.fragment.pan"), "global content").unwrap();
        fs::write(local_dir.join("private.fragment.pan"), "local content").unwrap();

        let ws = Workspace::resolve(tmp.path(), &cfg).unwrap();

        assert!(ws.global_fragment_names.contains("shared"));
        assert!(!ws.global_fragment_names.contains("private"));
        let local = ws.module_fragment_names.get("default").unwrap();
        assert!(local.contains("private"));
        assert!(!local.contains("shared"));
        assert_eq!(ws.raw_fragments.len(), 2);
    }
}
