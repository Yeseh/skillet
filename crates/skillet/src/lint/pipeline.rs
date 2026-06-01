//! Phased lint pipeline: Phase 1 (parallel source scan) and Phase 2 (parallel
//! ref extraction).
//!
//! These two phases produce the pre-scanned data that all lint rules consume,
//! eliminating redundant file I/O inside individual rule implementations.

use crate::parse::SkillFrontmatter;
use crate::refs::{ParsedRefs};
use crate::tokens::count_tokens;
use rayon::prelude::*;
use sha2::Digest;
use std::path::PathBuf;

// ── Data model ────────────────────────────────────────────────────────────────

/// Whether a scanned source file is a skill source or a reference document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileType {
    /// The primary `.pan` skill source file.
    Skill,
    /// A file inside a skill's `reference/` subtree.
    ReferenceDocument,
}

/// A source file fully scanned in Phase 1 and enriched with refs in Phase 2.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Sequential index in the workspace-level `Vec<SourceFile>`.
    pub id: usize,
    /// File type.
    pub file_type: SourceFileType,
    /// Name of the owning skill (directory name).
    pub name: String,
    /// Absolute path to this source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's source directory.
    pub skill_dir: PathBuf,
    /// Absolute path to the skill's compiled output directory.
    pub skill_out_dir: PathBuf,
    /// Raw file content.
    pub raw: String,
    /// `"sha256:<hex>"` of the raw bytes — computed once in Phase 1.
    pub source_hash: String,
    /// Token count for this file's raw content — computed once in Phase 1.
    pub token_count: u32,
    /// Parsed YAML frontmatter (`Skill` files only; `None` on missing/parse error).
    pub frontmatter: Option<SkillFrontmatter>,
    /// Parse errors collected during Phase 1 frontmatter extraction.
    pub parse_errors: Vec<String>,
    /// Refs extracted from this file during Phase 2 (empty until Phase 2 runs).
    pub parsed_refs: ParsedRefs,
}

/// A workspace-level typed ref with source location.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum Ref {
    /// `skill::` cross-skill reference.
    Skill {
        value: String,
        file_id: usize,
        line: u32,
        col: u32,
    },
    /// `cmd::` shell command reference.
    Cmd {
        value: String,
        file_id: usize,
        line: u32,
        col: u32,
    },
    /// `ref::` path reference or non-URL markdown link.
    PathRef {
        value: String,
        file_id: usize,
        line: u32,
        col: u32,
    },
    /// `var::` workspace variable reference.
    Var {
        value: String,
        file_id: usize,
        line: u32,
        col: u32,
    },
    /// `env::` environment variable reference.
    Env {
        value: String,
        file_id: usize,
        line: u32,
        col: u32,
    },
    /// Untyped backtick classified by the Layer 3 heuristic.
    Untyped {
        value: String,
        inferred_kind: &'static str,
        file_id: usize,
        line: u32,
        col: u32,
    },
}

/// All refs extracted across the workspace in Phase 2.
pub type AllRefs = Vec<Ref>;

// ── Phase 1: Parallel source scan ─────────────────────────────────────────────

/// A pre-loaded source input for Phase 1 scanning.
///
/// The CLI reads all files from disk and populates this struct; the library
/// only operates on in-memory data.
#[derive(Debug, Clone)]
pub struct SourceInput {
    /// Skill name (directory name).
    pub name: String,
    /// Absolute path to the `.pan` source file.
    pub source_path: PathBuf,
    /// Absolute path to the skill's source directory.
    pub skill_dir: PathBuf,
    /// Absolute path to the skill's compiled output directory.
    pub skill_out_dir: PathBuf,
    /// File content (already read from disk by the caller).
    pub content: String,
    /// Reference documents: (path, content) pairs from `reference/` subdir.
    pub reference_docs: Vec<(PathBuf, String)>,
}


/// Phase 1 — scans all pre-loaded skill sources in parallel via
/// `rayon::par_iter`.
///
/// Computes SHA-256 hashes, counts tokens, and parses frontmatter for `Skill`
/// files. Assigns sequential IDs after collection.
pub fn scan_sources(inputs: &[SourceInput], tokenizer: &str) -> Vec<SourceFile> {
    let mut files: Vec<SourceFile> = inputs
        .par_iter()
        .flat_map(|input| scan_input(input, tokenizer))
        .collect();
    // Assign stable sequential IDs after parallel flat_map.
    for (i, sf) in files.iter_mut().enumerate() {
        sf.id = i;
    }
    files
}

/// Scans the pre-loaded `.pan` content and any reference docs for one skill.
fn scan_input(input: &SourceInput, tokenizer: &str) -> Vec<SourceFile> {
    let mut files = Vec::new();

    // Primary .pan source file.
    let sf = scan_content(
        &input.name,
        &input.source_path,
        &input.skill_dir,
        &input.skill_out_dir,
        &input.content,
        SourceFileType::Skill,
        tokenizer,
    );
    files.push(sf);

    // Reference documents.
    for (path, content) in &input.reference_docs {
        let sf = scan_content(
            &input.name,
            path,
            &input.skill_dir,
            &input.skill_out_dir,
            content,
            SourceFileType::ReferenceDocument,
            tokenizer,
        );
        files.push(sf);
    }

    files
}

fn scan_content(
    name: &str,
    path: &PathBuf,
    skill_dir: &PathBuf,
    skill_out_dir: &PathBuf,
    raw: &str,
    file_type: SourceFileType,
    tokenizer: &str,
) -> SourceFile {
    let source_hash = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(raw.as_bytes()))
    );
    let token_count = count_tokens(raw, tokenizer);

    let (frontmatter, parse_errors) = if matches!(file_type, SourceFileType::Skill) {
        match crate::parse::parse_frontmatter(raw) {
            Ok(fm) => (fm, vec![]),
            Err(e) => (None, vec![e.to_string()]),
        }
    } else {
        (None, vec![])
    };

    SourceFile {
        id: 0, // Reassigned by scan_sources after collection.
        file_type,
        name: name.to_string(),
        source_path: path.clone(),
        skill_dir: skill_dir.clone(),
        skill_out_dir: skill_out_dir.clone(),
        raw: raw.to_string(),
        source_hash,
        token_count,
        frontmatter,
        parse_errors,
        parsed_refs: ParsedRefs::default(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_input(tmp: &TempDir, name: &str, content: &str) -> SourceInput {
        let skill_dir = tmp.path().join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        SourceInput {
            name: name.to_string(),
            source_path,
            skill_dir,
            skill_out_dir: tmp.path().join("skills").join(name),
            content: content.to_string(),
            reference_docs: vec![],
        }
    }

    #[test]
    fn scan_sources_reads_raw_and_computes_hash() {
        let tmp = TempDir::new().unwrap();
        let input = make_input(&tmp, "alpha", "---\nname: alpha\ndescription: x\n---\n");
        let files = scan_sources(&[input], "cl100k_base");
        assert_eq!(files.len(), 1);
        let sf = &files[0];
        assert_eq!(sf.name, "alpha");
        assert!(sf.source_hash.starts_with("sha256:"));
        assert!(!sf.raw.is_empty());
        assert_eq!(sf.file_type, SourceFileType::Skill);
    }

    #[test]
    fn scan_sources_parses_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let input = make_input(
            &tmp,
            "beta",
            "---\nname: beta\ndescription: a beta skill\n---\n",
        );
        let files = scan_sources(&[input], "cl100k_base");
        let fm = files[0].frontmatter.as_ref().expect("frontmatter");
        assert_eq!(fm.name.as_deref(), Some("beta"));
        assert_eq!(fm.description.as_deref(), Some("a beta skill"));
    }

    #[test]
    fn scan_sources_discovers_reference_documents() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("src/skills/gamma");
        fs::create_dir_all(&skill_dir).unwrap();
        let input = SourceInput {
            name: "gamma".to_string(),
            source_path: skill_dir.join("gamma.pan"),
            skill_dir: skill_dir.clone(),
            skill_out_dir: tmp.path().join("skills/gamma"),
            content: "---\nname: gamma\ndescription: g\n---\n".to_string(),
            reference_docs: vec![(
                skill_dir.join("reference/guide.md"),
                "# Guide\n".to_string(),
            )],
        };

        let files = scan_sources(&[input], "cl100k_base");
        assert_eq!(files.len(), 2);
        let ref_file = files
            .iter()
            .find(|f| f.file_type == SourceFileType::ReferenceDocument);
        assert!(ref_file.is_some());
    }

    #[test]
    fn scan_sources_assigns_sequential_ids() {
        let tmp = TempDir::new().unwrap();
        let a = make_input(&tmp, "aaa", "---\nname: aaa\ndescription: x\n---\n");
        let b = make_input(&tmp, "bbb", "---\nname: bbb\ndescription: y\n---\n");
        let files = scan_sources(&[a, b], "cl100k_base");
        let ids: Vec<usize> = files.iter().map(|sf| sf.id).collect();
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }
}
