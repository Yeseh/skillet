use std::{fmt::{Display, Pointer}, path::PathBuf};

use serde::Deserialize;

use crate::workspace::{CompiledArtefact};

/// The YAML frontmatter of a `.pan` skill source file.
///
/// All fields are `Option` so callers can report missing-field errors rather
/// than receiving a parse failure from the deserializer.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    /// Skill identifier — must match the containing directory name.
    pub name: Option<String>,
    /// Short description shown in discovery-token reporting.
    pub description: Option<String>,
}

/// A discovered skill within the workspace.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Skill {
    /// The name of the skill
    pub name: String,
    /// Absolute path to the `{name}.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's source directory.
    pub src_dir: PathBuf,
    /// Absolute path to the skill's output directory.
    pub target_dir: PathBuf,
    /// Scripts discovered within this skill.
    pub scripts: Vec<Script>,
    /// Reference `.pan` files discovered within this skill.
    pub references: Vec<Reference>,
}

impl CompiledArtefact for Skill {
    fn check(&self, ws: &super::Workspace) -> anyhow::Result<Vec<crate::compiler::CompileDiag>> {
        todo!()
    }

    fn compile(&self, ws: &super::Workspace) -> anyhow::Result<Option<String>, Vec<crate::compiler::CompileDiag>> {
        todo!()
    }
}

/// A script file associated with a skill.
/// Scripts are not compiled but directly copied when encountered, should be done in emite step of skill
#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    /// Path relative to the skill directory (e.g. `scripts/setup.sh`).
    pub relative_path: String,
    /// Absolute path to the script file.
    pub absolute_path: PathBuf,
}


/// A reference `.pan` file associated with a skill.
/// References can be .pan files or .md, should be compiled when .pan OR (.md and parse_md option is true)
#[derive(Debug, Clone)]
pub struct Reference {
    pub name: String,
    /// Path relative to the skill directory (e.g. `references/api/types.pan`).
    pub relative_path: String,
    /// Absolute path to the reference file.
    pub absolute_path: PathBuf,
}

impl Display for Skill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "skills/{}", self.name)
    }
}

impl Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "refs/{}", self.name)
    }
}

impl Display for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scripts/{}", self.name)
    }
}
