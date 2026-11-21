use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use skillet::config::SkilletConfig;
use skillet::navigation::{AgentDoc, FragmentDoc, SkillDoc, WorkspaceSnapshot};
use skillet::workspace::skill::Reference;
use skillet::workspace::Workspace;
use walkdir::WalkDir;

/// Workspace data and unsaved buffers used by LSP requests.
pub struct LspState {
    open_docs: HashMap<PathBuf, String>,
    client_supports_rename_file: bool,
}

impl LspState {
    pub fn new() -> Self {
        Self {
            open_docs: HashMap::new(),
            client_supports_rename_file: false,
        }
    }

    pub fn set_client_supports_rename_file(&mut self, supported: bool) {
        self.client_supports_rename_file = supported;
    }

    pub fn client_supports_rename_file(&self) -> bool {
        self.client_supports_rename_file
    }

    pub fn did_open(&mut self, path: PathBuf, text: String) {
        self.open_docs.insert(path, text);
    }

    pub fn did_change(&mut self, path: PathBuf, text: String) {
        self.open_docs.insert(path, text);
    }

    pub fn did_close(&mut self, path: &Path) {
        self.open_docs.remove(path);
    }

    pub fn did_save(&mut self, path: PathBuf, text: Option<String>) {
        if let Some(text) = text {
            self.open_docs.insert(path, text);
        } else {
            self.open_docs.remove(&path);
        }
    }

    /// Builds a navigation snapshot for the workspace containing `doc_path`.
    pub fn build_snapshot_for(&self, doc_path: &Path) -> Result<Option<WorkspaceSnapshot>> {
        let Some(root) = find_workspace_root(doc_path) else {
            return Ok(None);
        };

        let config_path = root.join("skillet.toml");
        let config = SkilletConfig::load(&root)
            .with_context(|| format!("failed to load '{}'", config_path.display()))?;
        let config_raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read '{}'", config_path.display()))?;
        let workspace = Workspace::resolve(&root, &config)
            .with_context(|| format!("failed to resolve workspace '{}'", root.display()))?;

        let mut skills = workspace
            .skills
            .values()
            .map(|skill| {
                let source = self.read_document(&skill.source_path)?;
                let references = skill
                    .references
                    .iter()
                    .map(|reference| Reference {
                        name: reference.name.clone(),
                        relative_path: reference.relative_path.clone(),
                        absolute_path: reference.absolute_path.clone(),
                    })
                    .collect();

                Ok(SkillDoc {
                    name: skill.name.clone(),
                    plugin: skill.plugin.clone(),
                    src_dir: skill
                        .src_dir
                        .parent()
                        .context("skill source directory has no plugin directory")?
                        .to_path_buf(),
                    path: skill.source_path.clone(),
                    source,
                    references,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        skills.sort_by(|left, right| left.path.cmp(&right.path));

        let mut agents = workspace
            .agents
            .values()
            .map(|agent| {
                Ok(AgentDoc {
                    name: agent.name.clone(),
                    plugin: agent.plugin.clone(),
                    path: agent.source_path.clone(),
                    source: self.read_document(&agent.source_path)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        agents.sort_by(|left, right| left.path.cmp(&right.path));

        let fragments = collect_fragments(&root, &config)?;

        Ok(Some(WorkspaceSnapshot::new(
            root,
            config_path,
            config_raw,
            skills,
            agents,
            fragments,
        )))
    }

    fn read_document(&self, path: &Path) -> Result<String> {
        if let Some(text) = self.open_docs.get(path) {
            return Ok(text.clone());
        }
        fs::read_to_string(path).with_context(|| format!("failed to read '{}'", path.display()))
    }
}

fn find_workspace_root(doc_path: &Path) -> Option<PathBuf> {
    let mut directory = doc_path.parent()?;

    loop {
        if directory.join("skillet.toml").is_file() {
            return Some(directory.to_path_buf());
        }
        directory = directory.parent()?;
    }
}

fn collect_fragments(root: &Path, config: &SkilletConfig) -> Result<Vec<FragmentDoc>> {
    let mut fragments = Vec::new();

    if let Some(directory) = &config.workspace.fragments_dir {
        collect_fragments_in(&root.join(directory), None, &mut fragments)?;
    }

    for (plugin, plugin_config) in &config.plugins {
        if let Some(directory) = &plugin_config.fragments_dir {
            collect_fragments_in(&root.join(directory), Some(plugin.as_str()), &mut fragments)?;
        }
    }

    fragments.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(fragments)
}

fn collect_fragments_in(
    directory: &Path,
    plugin: Option<&str>,
    fragments: &mut Vec<FragmentDoc>,
) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(directory).min_depth(1).max_depth(1) {
        let entry = entry.with_context(|| format!("failed to read '{}'", directory.display()))?;
        let path = entry.into_path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".fragment.pan"))
        else {
            continue;
        };

        fragments.push(FragmentDoc {
            name: name.to_string(),
            path,
            plugin: plugin.map(str::to_string),
        });
    }

    Ok(())
}
