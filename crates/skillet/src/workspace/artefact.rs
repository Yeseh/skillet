use std::path::PathBuf;

pub enum ArtefactKind {
    Agent,
    Skill,
    Reference,
}

pub struct Artefact {
    pub kind: ArtefactKind,
    pub source_path: PathBuf,
}
