use std::path::PathBuf;

/// Classifies what kind of artefact a `.pan` file represents.
pub enum ArtefactKind {
    /// A compiled agent definition.
    Agent,
    /// A compiled skill definition.
    Skill,
    /// A compiled reference document.
    Reference,
}

/// A single compilable artefact discovered in the workspace.
pub struct Artefact {
    /// The artefact's kind.
    pub kind: ArtefactKind,
    /// Absolute path to the source `.pan` file.
    pub source_path: PathBuf,
}
