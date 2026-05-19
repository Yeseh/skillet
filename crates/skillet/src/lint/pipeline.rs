//! Phased lint pipeline: Phase 1 (parallel source scan) and Phase 2 (parallel
//! ref extraction).
//!
//! These two phases produce the pre-scanned data that all lint rules consume,
//! eliminating redundant file I/O inside individual rule implementations.

use crate::parse::SkillFrontmatter;
use crate::refs::{ParsedRefs, RefKind};
use crate::tokens::count_tokens;
use crate::workspace::SkillSource;
use rayon::prelude::*;
use sha2::Digest;
use std::path::{Path, PathBuf};

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

/// Phase 1 — scans all skill sources (and their `reference/` subdirs) in
/// parallel via `rayon::par_iter`.
///
/// Reads each file, computes its SHA-256 hash, counts tokens, and parses
/// frontmatter for `Skill` files. Assigns sequential IDs after collection.
pub fn scan_sources(sources: &[SkillSource], tokenizer: &str) -> Vec<SourceFile> {
    let mut files: Vec<SourceFile> = sources
        .par_iter()
        .flat_map(|src| scan_skill_files(src, tokenizer))
        .collect();
    // Assign stable sequential IDs after parallel flat_map.
    for (i, sf) in files.iter_mut().enumerate() {
        sf.id = i;
    }
    files
}

/// Scans the `.pan` source file and any `reference/` docs for one skill.
fn scan_skill_files(src: &SkillSource, tokenizer: &str) -> Vec<SourceFile> {
    let mut files = Vec::new();

    // Primary .pan source file.
    files.push(
        match read_and_scan(src, &src.source_path, SourceFileType::Skill, tokenizer) {
            Ok(sf) => sf,
            Err(e) => SourceFile {
                id: 0,
                file_type: SourceFileType::Skill,
                name: src.name.clone(),
                source_path: src.source_path.clone(),
                skill_dir: src.skill_dir.clone(),
                skill_out_dir: src.skill_out_dir.clone(),
                raw: String::new(),
                source_hash: String::new(),
                token_count: 0,
                frontmatter: None,
                parse_errors: vec![format!("cannot read source: {e}")],
                parsed_refs: ParsedRefs::default(),
            },
        },
    );

    // Reference documents under `{skill_dir}/reference/`.
    let ref_dir = src.skill_dir.join("reference");
    if ref_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&ref_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(sf) =
                        read_and_scan(src, &path, SourceFileType::ReferenceDocument, tokenizer)
                    {
                        files.push(sf);
                    }
                }
            }
        }
    }

    files
}

fn read_and_scan(
    src: &SkillSource,
    path: &Path,
    file_type: SourceFileType,
    tokenizer: &str,
) -> anyhow::Result<SourceFile> {
    let raw = std::fs::read_to_string(path)?;
    let source_hash = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(raw.as_bytes()))
    );
    let token_count = count_tokens(&raw, tokenizer);

    let (frontmatter, parse_errors) = if matches!(file_type, SourceFileType::Skill) {
        match crate::parse::parse_frontmatter(&raw) {
            Ok(fm) => (fm, vec![]),
            Err(e) => (None, vec![e.to_string()]),
        }
    } else {
        (None, vec![])
    };

    Ok(SourceFile {
        id: 0, // Reassigned by scan_sources after collection.
        file_type,
        name: src.name.clone(),
        source_path: path.to_path_buf(),
        skill_dir: src.skill_dir.clone(),
        skill_out_dir: src.skill_out_dir.clone(),
        raw,
        source_hash,
        token_count,
        frontmatter,
        parse_errors,
        parsed_refs: ParsedRefs::default(),
    })
}

// ── Phase 2: Parallel ref extraction ──────────────────────────────────────────

/// Phase 2 — extracts refs from all source files in parallel via
/// `rayon::par_iter`.
///
/// Takes ownership of `source_files`, populates `parsed_refs` on each entry,
/// and returns the updated files plus a flat workspace-wide [`AllRefs`].
pub fn extract_refs(
    mut source_files: Vec<SourceFile>,
    skill_names: &[&str],
) -> (Vec<SourceFile>, AllRefs) {
    // Parallel ref extraction — one ParsedRefs per file.
    let extracted: Vec<ParsedRefs> = source_files
        .par_iter()
        .map(|sf| {
            if sf.parse_errors.is_empty() && !sf.raw.is_empty() {
                ParsedRefs::extract(&sf.raw, skill_names)
            } else {
                ParsedRefs::default()
            }
        })
        .collect();

    // Assign back (sequential pass).
    for (sf, parsed) in source_files.iter_mut().zip(extracted) {
        sf.parsed_refs = parsed;
    }

    let all_refs = build_all_refs(&source_files);
    (source_files, all_refs)
}

/// Flattens per-file `parsed_refs` into a workspace-wide [`AllRefs`].
fn build_all_refs(source_files: &[SourceFile]) -> AllRefs {
    let mut all_refs = AllRefs::new();
    for sf in source_files {
        let file_id = sf.id;
        for tr in &sf.parsed_refs.typed {
            let r = match tr.kind {
                RefKind::Ref => Ref::PathRef {
                    value: tr.value.clone(),
                    file_id,
                    line: tr.line,
                    col: tr.col,
                },
                RefKind::Cmd => Ref::Cmd {
                    value: tr.value.clone(),
                    file_id,
                    line: tr.line,
                    col: tr.col,
                },
                RefKind::Skill => Ref::Skill {
                    value: tr.value.clone(),
                    file_id,
                    line: tr.line,
                    col: tr.col,
                },
                RefKind::Var => Ref::Var {
                    value: tr.value.clone(),
                    file_id,
                    line: tr.line,
                    col: tr.col,
                },
                RefKind::Env => Ref::Env {
                    value: tr.value.clone(),
                    file_id,
                    line: tr.line,
                    col: tr.col,
                },
            };
            all_refs.push(r);
        }
        for link in &sf.parsed_refs.links {
            if !link.is_url {
                all_refs.push(Ref::PathRef {
                    value: link.target.clone(),
                    file_id,
                    line: link.line,
                    col: link.col,
                });
            }
        }
        for u in &sf.parsed_refs.untyped {
            all_refs.push(Ref::Untyped {
                value: u.content.clone(),
                inferred_kind: u.inferred_kind,
                file_id,
                line: u.line,
                col: u.col,
            });
        }
    }
    all_refs
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::SkillSource;
    use std::fs;
    use tempfile::TempDir;

    fn make_skill(tmp: &TempDir, name: &str, content: &str) -> SkillSource {
        let skill_dir = tmp.path().join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        fs::write(&source_path, content).unwrap();
        SkillSource {
            name: name.to_string(),
            source_path,
            skill_dir,
            skill_out_dir: tmp.path().join("skills").join(name),
        }
    }

    #[test]
    fn scan_sources_reads_raw_and_computes_hash() {
        let tmp = TempDir::new().unwrap();
        let src = make_skill(&tmp, "alpha", "---\nname: alpha\ndescription: x\n---\n");
        let files = scan_sources(&[src], "cl100k_base");
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
        let src = make_skill(
            &tmp,
            "beta",
            "---\nname: beta\ndescription: a beta skill\n---\n",
        );
        let files = scan_sources(&[src], "cl100k_base");
        let fm = files[0].frontmatter.as_ref().expect("frontmatter");
        assert_eq!(fm.name.as_deref(), Some("beta"));
        assert_eq!(fm.description.as_deref(), Some("a beta skill"));
    }

    #[test]
    fn scan_sources_discovers_reference_documents() {
        let tmp = TempDir::new().unwrap();
        let src = make_skill(&tmp, "gamma", "---\nname: gamma\ndescription: g\n---\n");
        let ref_dir = src.skill_dir.join("reference");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(ref_dir.join("guide.md"), "# Guide\n").unwrap();

        let files = scan_sources(&[src], "cl100k_base");
        assert_eq!(files.len(), 2);
        let ref_file = files
            .iter()
            .find(|f| f.file_type == SourceFileType::ReferenceDocument);
        assert!(ref_file.is_some());
    }

    #[test]
    fn scan_sources_assigns_sequential_ids() {
        let tmp = TempDir::new().unwrap();
        let a = make_skill(&tmp, "aaa", "---\nname: aaa\ndescription: x\n---\n");
        let b = make_skill(&tmp, "bbb", "---\nname: bbb\ndescription: y\n---\n");
        let files = scan_sources(&[a, b], "cl100k_base");
        let ids: Vec<usize> = files.iter().map(|sf| sf.id).collect();
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }

    #[test]
    fn extract_refs_populates_typed_refs() {
        let tmp = TempDir::new().unwrap();
        let src = make_skill(
            &tmp,
            "delta",
            "---\nname: delta\ndescription: x\n---\n\nSee `ref::helper.sh`\n",
        );
        let files = scan_sources(&[src], "cl100k_base");
        let (files, all_refs) = extract_refs(files, &["delta"]);
        assert!(!files[0].parsed_refs.typed.is_empty());
        let path_refs: Vec<_> = all_refs
            .iter()
            .filter(|r| matches!(r, Ref::PathRef { .. }))
            .collect();
        assert!(!path_refs.is_empty());
    }
}
