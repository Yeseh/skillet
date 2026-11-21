//! Pure symbol lookup, definition resolution, and rename planning.
//!
//! Callers assemble a [`WorkspaceSnapshot`] from pre-read source files. This
//! module does not access the filesystem; returned text edits and file renames
//! describe operations for the caller to apply.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::agent::AgentName;
use crate::compiler::compile::body_start_offset;
use crate::compiler::parse::{Node, PanParse, RefKind as PanRefKind};
use crate::compiler::PanSource;
use crate::workspace::skill::Reference;

/// Half-open byte range within a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}

/// Kind of navigable or renameable symbol at a cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// `skill::` reference or skill definition.
    Skill,
    /// `ref::` reference.
    Reference,
    /// Fragment include.
    Fragment,
    /// `agent::` reference or agent definition.
    Agent,
    /// `var::` reference.
    Var,
    /// `env::` reference.
    Env,
    /// `cmd::` reference.
    Cmd,
    /// `url::` reference.
    Url,
}

/// A symbol located in a `.pan` document.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolAtCursor {
    /// Kind of the symbol.
    pub kind: SymbolKind,
    /// Symbol name, without directive delimiters.
    pub name: String,
    /// Full token range, including its delimiters.
    pub full_range: ByteRange,
    /// Editable identifier range.
    pub value_range: ByteRange,
}

/// Target of a go-to-definition request.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTarget {
    /// Source file that defines the symbol.
    pub path: PathBuf,
    /// Optional byte range inside `path`; `None` means the top of the file.
    pub target_range: Option<ByteRange>,
}

/// A single text replacement in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// Range to replace.
    pub range: ByteRange,
    /// Replacement text.
    pub new_text: String,
}

/// All text edits targeting a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTextEdits {
    /// Source file to edit.
    pub path: PathBuf,
    /// Replacements in source order.
    pub edits: Vec<TextEdit>,
}

/// A file or directory rename to execute after text edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRename {
    /// Existing path.
    pub old_path: PathBuf,
    /// New path.
    pub new_path: PathBuf,
}

/// Complete plan for renaming a workspace symbol.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    /// Text replacements, grouped by source file.
    pub text_edits: Vec<FileTextEdits>,
    /// File and directory renames, in execution order.
    pub file_renames: Vec<FileRename>,
}

/// Typed errors returned while preparing or planning a rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    /// The cursor is not on a symbol.
    NoSymbolAtCursor,
    /// The symbol kind cannot be renamed.
    NotRenameable {
        /// Kind of the symbol.
        kind: SymbolKind,
        /// Actionable reason that rename is unavailable.
        reason: String,
    },
    /// A renameable symbol has no definition in this snapshot.
    UnresolvedTarget {
        /// Kind of the symbol.
        kind: SymbolKind,
        /// Name of the unresolved symbol.
        name: String,
    },
    /// The requested name violates the symbol's naming rules.
    InvalidName {
        /// Kind of the symbol.
        kind: SymbolKind,
        /// Requested name.
        name: String,
        /// Reason that the name is invalid.
        reason: String,
    },
    /// The requested name already exists in the symbol's scope.
    AlreadyExists {
        /// Existing target path.
        path: PathBuf,
    },
    /// The requested name is the current name.
    NoOp,
}

/// Pre-read in-memory view of source files needed for navigation operations.
///
/// `SkillDoc::src_dir` is the owning plugin's source directory. A fragment with
/// `plugin: None` belongs to the workspace-global fragments directory;
/// `Some(plugin)` marks a plugin-local fragment.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    /// Workspace root.
    pub root: PathBuf,
    /// Path to the workspace `skillet.toml`.
    pub config_path: PathBuf,
    /// Raw contents of `skillet.toml`.
    pub config_raw: String,
    /// Pre-read skill sources and their reference-file inventory.
    pub skills: Vec<SkillDoc>,
    /// Pre-read agent sources.
    pub agents: Vec<AgentDoc>,
    /// Fragment paths and scopes.
    pub fragments: Vec<FragmentDoc>,
}

/// A pre-read skill source and its reference-file inventory.
#[derive(Debug, Clone)]
pub struct SkillDoc {
    /// Skill name.
    pub name: String,
    /// Owning plugin name.
    pub plugin: String,
    /// Owning plugin's source directory.
    pub src_dir: PathBuf,
    /// Skill source path, normally `{src_dir}/{name}/{name}.pan`.
    pub path: PathBuf,
    /// Raw skill source text.
    pub source: String,
    /// Reference files available to this skill, including `.pan` and `.md`.
    pub references: Vec<Reference>,
}

/// A pre-read agent source.
#[derive(Debug, Clone)]
pub struct AgentDoc {
    /// Agent name.
    pub name: String,
    /// Owning plugin name.
    pub plugin: String,
    /// Agent source path.
    pub path: PathBuf,
    /// Raw agent source text.
    pub source: String,
}

/// A fragment path and its scope.
#[derive(Debug, Clone)]
pub struct FragmentDoc {
    /// Fragment name without `.fragment.pan`.
    pub name: String,
    /// Fragment source path.
    pub path: PathBuf,
    /// Owning plugin, or `None` for a workspace-global fragment.
    pub plugin: Option<String>,
}

#[derive(Clone, Copy)]
enum SourceKind {
    Skill,
    Agent,
}

#[derive(Clone, Copy)]
struct SourceDocument<'a> {
    kind: SourceKind,
    plugin: &'a str,
    path: &'a Path,
    source: &'a str,
}

impl WorkspaceSnapshot {
    /// Creates a snapshot from caller-provided, pre-read workspace data.
    pub fn new(
        root: PathBuf,
        config_path: PathBuf,
        config_raw: String,
        skills: Vec<SkillDoc>,
        agents: Vec<AgentDoc>,
        fragments: Vec<FragmentDoc>,
    ) -> Self {
        Self {
            root,
            config_path,
            config_raw,
            skills,
            agents,
            fragments,
        }
    }

    fn document(&self, path: &Path) -> Option<SourceDocument<'_>> {
        self.skills
            .iter()
            .find(|doc| doc.path == path)
            .map(|doc| SourceDocument {
                kind: SourceKind::Skill,
                plugin: &doc.plugin,
                path: &doc.path,
                source: &doc.source,
            })
            .or_else(|| {
                self.agents
                    .iter()
                    .find(|doc| doc.path == path)
                    .map(|doc| SourceDocument {
                        kind: SourceKind::Agent,
                        plugin: &doc.plugin,
                        path: &doc.path,
                        source: &doc.source,
                    })
            })
    }

    fn owner_skill(&self, path: &Path) -> Option<&SkillDoc> {
        self.skills.iter().find(|doc| doc.path == path)
    }
}

/// Finds a parsed symbol whose full token contains `byte_offset`.
///
/// Body positions retain their offsets in the original document, including
/// frontmatter. Tokens in escaped bodies and fenced code blocks are ignored.
/// A cursor on the scalar value of a skill or agent `name:` field resolves to
/// that definition.
pub fn symbol_at_offset(
    snapshot: &WorkspaceSnapshot,
    path: &Path,
    byte_offset: usize,
) -> Option<SymbolAtCursor> {
    let document = snapshot.document(path)?;

    if let Some((name, value_range)) = frontmatter_name(document.source) {
        if value_range.start <= byte_offset && byte_offset < value_range.end {
            let kind = match document.kind {
                SourceKind::Skill => SymbolKind::Skill,
                SourceKind::Agent => SymbolKind::Agent,
            };
            return Some(SymbolAtCursor {
                kind,
                name,
                full_range: value_range,
                value_range,
            });
        }
    }

    let (body_start, body, nodes) = parse_body(document.source);
    let fences = fenced_code_ranges(body);
    nodes.iter().find_map(|node| {
        let (kind, name, local_full, value_range) = parsed_symbol(node, body)?;
        let full_range = ByteRange {
            start: body_start + local_full.start,
            end: body_start + local_full.end,
        };
        if byte_offset < full_range.start || byte_offset >= full_range.end {
            return None;
        }
        if overlaps_any(local_full, &fences) {
            return None;
        }
        Some(SymbolAtCursor {
            kind,
            name,
            full_range,
            value_range: ByteRange {
                start: body_start + value_range.start,
                end: body_start + value_range.end,
            },
        })
    })
}

/// Resolves the symbol at `byte_offset` to its definition, if it has one.
///
/// `skill::` lookups stay within the owning plugin. References are resolved
/// only from skill sources; agents do not own reference directories. Local
/// fragments shadow workspace-global fragments.
pub fn resolve_definition(
    snapshot: &WorkspaceSnapshot,
    path: &Path,
    byte_offset: usize,
) -> Option<DefinitionTarget> {
    let symbol = symbol_at_offset(snapshot, path, byte_offset)?;
    let document = snapshot.document(path)?;

    match symbol.kind {
        SymbolKind::Skill => snapshot
            .skills
            .iter()
            .find(|skill| skill.name == symbol.name && skill.plugin == document.plugin)
            .map(|skill| definition(&skill.path)),
        SymbolKind::Reference => {
            let skill = snapshot.owner_skill(path)?;
            let references = skill
                .references
                .iter()
                .filter(|reference| reference.name == symbol.name)
                .collect::<Vec<_>>();
            (references.len() == 1).then(|| definition(&references[0].absolute_path))
        }
        SymbolKind::Fragment => resolve_fragment(snapshot, document.plugin, &symbol.name),
        SymbolKind::Agent => snapshot
            .agents
            .iter()
            .find(|agent| agent.name == symbol.name)
            .map(|agent| definition(&agent.path)),
        SymbolKind::Var => {
            config_key_range(snapshot, "vars", &symbol.name).map(|range| DefinitionTarget {
                path: snapshot.config_path.clone(),
                target_range: Some(range),
            })
        }
        SymbolKind::Env => {
            config_key_range(snapshot, "env", &symbol.name).map(|range| DefinitionTarget {
                path: snapshot.config_path.clone(),
                target_range: Some(range),
            })
        }
        SymbolKind::Cmd | SymbolKind::Url => None,
    }
}

/// Checks whether the symbol at `byte_offset` can be renamed.
///
/// External and unresolved symbols are rejected because a rename plan must
/// target source files represented in the snapshot.
pub fn prepare_rename(
    snapshot: &WorkspaceSnapshot,
    path: &Path,
    byte_offset: usize,
) -> Result<SymbolAtCursor, RenameError> {
    let symbol =
        symbol_at_offset(snapshot, path, byte_offset).ok_or(RenameError::NoSymbolAtCursor)?;
    match symbol.kind {
        SymbolKind::Skill | SymbolKind::Reference | SymbolKind::Fragment | SymbolKind::Agent => {
            let Some(target) = resolve_definition(snapshot, path, byte_offset) else {
                return Err(RenameError::UnresolvedTarget {
                    kind: symbol.kind,
                    name: symbol.name,
                });
            };
            let document = snapshot.document(path);
            if !target_is_workspace_source(snapshot, document, &target, &symbol) {
                return Err(RenameError::NotRenameable {
                    kind: symbol.kind,
                    reason: "the definition is outside a workspace source directory".to_string(),
                });
            }
        }
        kind => {
            return Err(RenameError::NotRenameable {
                kind,
                reason: match kind {
                    SymbolKind::Cmd => "command references do not have a source definition",
                    SymbolKind::Url => "URLs are external and cannot be renamed",
                    SymbolKind::Var => "workspace variables cannot be renamed in this ticket",
                    SymbolKind::Env => {
                        "environment variable declarations cannot be renamed in this ticket"
                    }
                    _ => unreachable!(),
                }
                .to_string(),
            });
        }
    }
    Ok(symbol)
}

fn target_is_workspace_source(
    snapshot: &WorkspaceSnapshot,
    document: Option<SourceDocument<'_>>,
    target: &DefinitionTarget,
    symbol: &SymbolAtCursor,
) -> bool {
    let Some(document) = document.filter(|document| document.path.starts_with(&snapshot.root))
    else {
        return false;
    };
    if !target.path.starts_with(&snapshot.root) {
        return false;
    }
    let document_is_source = match document.kind {
        SourceKind::Skill => snapshot
            .skills
            .iter()
            .any(|skill| skill.path == document.path && skill.path.starts_with(&skill.src_dir)),
        SourceKind::Agent => true,
    };
    if !document_is_source {
        return false;
    }

    match symbol.kind {
        SymbolKind::Skill => snapshot.skills.iter().any(|skill| {
            skill.plugin == document.plugin
                && skill.name == symbol.name
                && skill.path == target.path
                && skill.path.starts_with(&skill.src_dir)
        }),
        SymbolKind::Reference => snapshot.owner_skill(document.path).is_some_and(|skill| {
            skill.path.starts_with(&skill.src_dir)
                && skill
                    .path
                    .parent()
                    .is_some_and(|skill_dir| target.path.starts_with(skill_dir))
        }),
        SymbolKind::Fragment => resolved_fragment(snapshot, document.plugin, &symbol.name)
            .is_some_and(|fragment| fragment.path == target.path),
        SymbolKind::Agent => snapshot
            .agents
            .iter()
            .any(|agent| agent.name == symbol.name && agent.path == target.path),
        _ => false,
    }
}

/// Plans text edits and source-path renames for a workspace symbol.
///
/// All paths and source text come from `snapshot`; no filesystem operations
/// occur. Text edits precede file renames, and the two skill-file renames are
/// ordered so the `.pan` file is renamed before its directory.
pub fn plan_rename(
    snapshot: &WorkspaceSnapshot,
    path: &Path,
    byte_offset: usize,
    new_name: &str,
) -> Result<RenamePlan, RenameError> {
    let symbol = prepare_rename(snapshot, path, byte_offset)?;
    if symbol.name == new_name {
        return Err(RenameError::NoOp);
    }
    validate_new_name(symbol.kind, new_name)?;

    let document = snapshot
        .document(path)
        .ok_or(RenameError::NoSymbolAtCursor)?;
    check_collision(snapshot, document, &symbol, new_name)?;

    let mut edits = BTreeMap::<PathBuf, Vec<TextEdit>>::new();
    match symbol.kind {
        SymbolKind::Skill => {
            for skill in &snapshot.skills {
                if skill.plugin == document.plugin {
                    collect_ref_edits(
                        &mut edits,
                        &skill.path,
                        &skill.source,
                        PanRefKind::Skill,
                        &symbol.name,
                        new_name,
                    );
                    if skill.name == symbol.name {
                        collect_frontmatter_edit(&mut edits, &skill.path, &skill.source, new_name);
                    }
                }
            }
            for agent in &snapshot.agents {
                if agent.plugin == document.plugin {
                    collect_ref_edits(
                        &mut edits,
                        &agent.path,
                        &agent.source,
                        PanRefKind::Skill,
                        &symbol.name,
                        new_name,
                    );
                }
            }
        }
        SymbolKind::Reference => {
            let owner =
                snapshot
                    .owner_skill(path)
                    .ok_or_else(|| RenameError::UnresolvedTarget {
                        kind: symbol.kind,
                        name: symbol.name.clone(),
                    })?;
            collect_ref_edits(
                &mut edits,
                &owner.path,
                &owner.source,
                PanRefKind::Reference,
                &symbol.name,
                new_name,
            );
        }
        SymbolKind::Fragment => {
            let target_fragment = resolved_fragment(snapshot, document.plugin, &symbol.name)
                .ok_or_else(|| RenameError::UnresolvedTarget {
                    kind: symbol.kind,
                    name: symbol.name.clone(),
                })?;
            for skill in &snapshot.skills {
                if fragment_resolves_to(snapshot, &skill.plugin, &symbol.name, target_fragment) {
                    collect_fragment_edits(
                        &mut edits,
                        &skill.path,
                        &skill.source,
                        &symbol.name,
                        new_name,
                    );
                }
            }
            for agent in &snapshot.agents {
                if fragment_resolves_to(snapshot, &agent.plugin, &symbol.name, target_fragment) {
                    collect_fragment_edits(
                        &mut edits,
                        &agent.path,
                        &agent.source,
                        &symbol.name,
                        new_name,
                    );
                }
            }
        }
        SymbolKind::Agent => {
            for skill in &snapshot.skills {
                collect_ref_edits(
                    &mut edits,
                    &skill.path,
                    &skill.source,
                    PanRefKind::Agent,
                    &symbol.name,
                    new_name,
                );
            }
            for agent in &snapshot.agents {
                collect_ref_edits(
                    &mut edits,
                    &agent.path,
                    &agent.source,
                    PanRefKind::Agent,
                    &symbol.name,
                    new_name,
                );
                if agent.name == symbol.name {
                    collect_frontmatter_edit(&mut edits, &agent.path, &agent.source, new_name);
                }
            }
        }
        _ => unreachable!("prepare_rename rejects non-renameable kinds"),
    }

    let text_edits = edits
        .into_iter()
        .map(|(path, mut edits)| {
            edits.sort_by_key(|edit| edit.range.start);
            FileTextEdits { path, edits }
        })
        .collect();
    let file_renames = file_renames(snapshot, document, &symbol, new_name);
    Ok(RenamePlan {
        text_edits,
        file_renames,
    })
}

fn definition(path: &Path) -> DefinitionTarget {
    DefinitionTarget {
        path: path.to_path_buf(),
        target_range: None,
    }
}

fn resolve_fragment(
    snapshot: &WorkspaceSnapshot,
    plugin: &str,
    name: &str,
) -> Option<DefinitionTarget> {
    resolved_fragment(snapshot, plugin, name).map(|fragment| definition(&fragment.path))
}

fn resolved_fragment<'a>(
    snapshot: &'a WorkspaceSnapshot,
    plugin: &str,
    name: &str,
) -> Option<&'a FragmentDoc> {
    snapshot
        .fragments
        .iter()
        .find(|fragment| fragment.name == name && fragment.plugin.as_deref() == Some(plugin))
        .or_else(|| {
            snapshot
                .fragments
                .iter()
                .find(|fragment| fragment.name == name && fragment.plugin.is_none())
        })
}

fn parsed_symbol(node: &Node, body: &str) -> Option<(SymbolKind, String, ByteRange, ByteRange)> {
    match node {
        Node::Ref {
            kind,
            value,
            source_range,
        } => {
            let (symbol_kind, prefix) = match kind {
                PanRefKind::Skill => (SymbolKind::Skill, "skill::"),
                PanRefKind::Reference => (SymbolKind::Reference, "ref::"),
                PanRefKind::Agent => (SymbolKind::Agent, "agent::"),
                PanRefKind::Cmd => (SymbolKind::Cmd, "cmd::"),
                PanRefKind::Url => (SymbolKind::Url, "url::"),
                PanRefKind::Var => (SymbolKind::Var, "var::"),
                PanRefKind::Env => (SymbolKind::Env, "env::"),
                PanRefKind::Path => return None,
            };
            let name = value.trim();
            let raw = &body[source_range.start as usize..source_range.end as usize];
            let value_start = source_range.start as usize + 1 + prefix.len();
            let leading = value.len() - value.trim_start().len();
            let start = value_start + leading;
            let end = start + name.len();
            raw.starts_with('`').then_some((
                symbol_kind,
                name.to_string(),
                ByteRange {
                    start: source_range.start as usize,
                    end: source_range.end as usize,
                },
                ByteRange { start, end },
            ))
        }
        Node::Fragment {
            value,
            source_range,
        } => {
            let name = value.trim();
            let raw = &body[source_range.start as usize..source_range.end as usize];
            let start = source_range.start as usize + 2 + value.len() - value.trim_start().len();
            let end = start + name.len();
            raw.starts_with("{>").then_some((
                SymbolKind::Fragment,
                name.to_string(),
                ByteRange {
                    start: source_range.start as usize,
                    end: source_range.end as usize,
                },
                ByteRange { start, end },
            ))
        }
        _ => None,
    }
}

fn collect_ref_edits(
    edits: &mut BTreeMap<PathBuf, Vec<TextEdit>>,
    path: &Path,
    source: &str,
    target_kind: PanRefKind,
    old_name: &str,
    new_name: &str,
) {
    let (body_start, body, nodes) = parse_body(source);
    let fences = fenced_code_ranges(body);
    for node in &nodes {
        if let Node::Ref {
            kind,
            value,
            source_range,
        } = node
        {
            if *kind != target_kind || value.trim() != old_name {
                continue;
            }
            let local = ByteRange {
                start: source_range.start as usize,
                end: source_range.end as usize,
            };
            if overlaps_any(local, &fences) {
                continue;
            }
            let prefix = match kind {
                PanRefKind::Skill => "skill::",
                PanRefKind::Reference => "ref::",
                PanRefKind::Agent => "agent::",
                PanRefKind::Cmd => "cmd::",
                PanRefKind::Path => "path::",
                PanRefKind::Url => "url::",
                PanRefKind::Var => "var::",
                PanRefKind::Env => "env::",
            };
            let start = body_start + source_range.start as usize + 1 + prefix.len() + value.len()
                - value.trim_start().len();
            add_edit(
                edits,
                path,
                TextEdit {
                    range: ByteRange {
                        start,
                        end: start + old_name.len(),
                    },
                    new_text: new_name.to_string(),
                },
            );
        }
    }
}

fn collect_fragment_edits(
    edits: &mut BTreeMap<PathBuf, Vec<TextEdit>>,
    path: &Path,
    source: &str,
    old_name: &str,
    new_name: &str,
) {
    let (body_start, body, nodes) = parse_body(source);
    let fences = fenced_code_ranges(body);
    for node in &nodes {
        if let Node::Fragment {
            value,
            source_range,
        } = node
        {
            if value.trim() != old_name {
                continue;
            }
            let local = ByteRange {
                start: source_range.start as usize,
                end: source_range.end as usize,
            };
            if overlaps_any(local, &fences) {
                continue;
            }
            let start = body_start + source_range.start as usize + 2 + value.len()
                - value.trim_start().len();
            add_edit(
                edits,
                path,
                TextEdit {
                    range: ByteRange {
                        start,
                        end: start + old_name.len(),
                    },
                    new_text: new_name.to_string(),
                },
            );
        }
    }
}

fn parse_body(source: &str) -> (usize, &str, Vec<Node>) {
    let bom_len = usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let body_start = bom_len + body_start_offset(&source[bom_len..]);
    let body = &source[body_start..];
    let parser_source = PanSource::new(body.to_string());
    let mut parser = PanParse::new(&parser_source);
    parser.parse();
    (body_start, body, parser.nodes)
}

fn collect_frontmatter_edit(
    edits: &mut BTreeMap<PathBuf, Vec<TextEdit>>,
    path: &Path,
    source: &str,
    new_name: &str,
) {
    if let Some((_, range)) = frontmatter_name(source) {
        add_edit(
            edits,
            path,
            TextEdit {
                range,
                new_text: new_name.to_string(),
            },
        );
    }
}

fn add_edit(edits: &mut BTreeMap<PathBuf, Vec<TextEdit>>, path: &Path, edit: TextEdit) {
    edits.entry(path.to_path_buf()).or_default().push(edit);
}

fn frontmatter_name(source: &str) -> Option<(String, ByteRange)> {
    let bom_len = usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let text = &source[bom_len..];
    if !text.starts_with("---") {
        return None;
    }
    let body_start = body_start_offset(text);
    if body_start == 0 {
        return None;
    }

    let mut line_start = 0;
    for line in text[..body_start].split_inclusive('\n') {
        let without_newline = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = without_newline.trim();
        if trimmed == "---" {
            line_start += line.len();
            continue;
        }
        let Some(colon) = without_newline.find(':') else {
            line_start += line.len();
            continue;
        };
        if without_newline[..colon].trim() != "name" {
            line_start += line.len();
            continue;
        }

        let raw_value = &without_newline[colon + 1..];
        let leading = raw_value.len() - raw_value.trim_start().len();
        let value = raw_value.trim_start();
        if value.is_empty() {
            return None;
        }
        let (value, quote_offset) = if let Some(quote @ ('"' | '\'')) = value.chars().next() {
            let closing = value[1..].find(quote)? + 1;
            (&value[1..closing], 1)
        } else {
            let scalar = value.split_once(" #").map_or(value, |(v, _)| v).trim_end();
            (scalar, 0)
        };
        if value.is_empty() {
            return None;
        }
        let start = bom_len + line_start + colon + 1 + leading + quote_offset;
        return Some((
            value.to_string(),
            ByteRange {
                start,
                end: start + value.len(),
            },
        ));
    }
    None
}

fn fenced_code_ranges(source: &str) -> Vec<ByteRange> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    let mut opening: Option<(u8, usize, usize)> = None;
    for line in source.split_inclusive('\n') {
        let line_content = line.strip_suffix('\n').unwrap_or(line);
        let indent = line_content.len() - line_content.trim_start_matches(' ').len();
        let trimmed = &line_content[indent..];
        if indent <= 3 {
            let bytes = trimmed.as_bytes();
            let fence_char = bytes.first().copied();
            if matches!(fence_char, Some(b'`' | b'~')) {
                let count = bytes
                    .iter()
                    .take_while(|&&byte| Some(byte) == fence_char)
                    .count();
                if count >= 3 {
                    if let Some((open_char, open_count, start)) = opening {
                        let trailing = &trimmed[count..];
                        if open_char == fence_char.unwrap()
                            && count >= open_count
                            && trailing.trim().is_empty()
                        {
                            ranges.push(ByteRange {
                                start,
                                end: offset + line.len(),
                            });
                            opening = None;
                        }
                    } else {
                        opening = Some((fence_char.unwrap(), count, offset));
                    }
                }
            }
        }
        offset += line.len();
    }
    if let Some((_, _, start)) = opening {
        ranges.push(ByteRange {
            start,
            end: source.len(),
        });
    }
    ranges
}

fn overlaps_any(range: ByteRange, others: &[ByteRange]) -> bool {
    others
        .iter()
        .any(|other| range.start < other.end && other.start < range.end)
}

fn config_key_range(snapshot: &WorkspaceSnapshot, section: &str, key: &str) -> Option<ByteRange> {
    let mut current_section = "";
    let mut offset = 0;
    for line in snapshot.config_raw.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
        } else if current_section == section {
            let without_comment = line.split_once('#').map_or(line, |(before, _)| before);
            if let Some((raw_key, _)) = without_comment.split_once('=') {
                let trimmed_key = raw_key.trim();
                let unquoted = trimmed_key
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        trimmed_key
                            .strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    })
                    .unwrap_or(trimmed_key);
                if unquoted == key {
                    let leading = raw_key.len() - raw_key.trim_start().len();
                    let start = offset + leading;
                    return Some(ByteRange {
                        start,
                        end: start + trimmed_key.len(),
                    });
                }
            }
        }
        offset += line.len();
    }
    None
}

fn fragment_resolves_to(
    snapshot: &WorkspaceSnapshot,
    plugin: &str,
    name: &str,
    target: &FragmentDoc,
) -> bool {
    resolved_fragment(snapshot, plugin, name).is_some_and(|fragment| fragment.path == target.path)
}

fn validate_new_name(kind: SymbolKind, name: &str) -> Result<(), RenameError> {
    let invalid = |reason: String| RenameError::InvalidName {
        kind,
        name: name.to_string(),
        reason,
    };
    match kind {
        SymbolKind::Skill | SymbolKind::Agent | SymbolKind::Fragment => {
            AgentName::parse(name).map_err(|error| invalid(error.to_string()))?;
        }
        SymbolKind::Reference => {
            let invalid_path = name.starts_with('/')
                || name.contains('\\')
                || name.ends_with(".pan")
                || name.ends_with(".md")
                || name
                    .split('/')
                    .any(|segment| segment.is_empty() || segment == "..");
            if invalid_path {
                return Err(invalid(
                    "use relative slash-separated segments without empty or '..' segments or a .pan/.md suffix"
                        .to_string(),
                ));
            }
        }
        _ => {
            return Err(RenameError::NotRenameable {
                kind,
                reason: "this symbol kind cannot be renamed".to_string(),
            });
        }
    }
    Ok(())
}

fn check_collision(
    snapshot: &WorkspaceSnapshot,
    document: SourceDocument<'_>,
    symbol: &SymbolAtCursor,
    new_name: &str,
) -> Result<(), RenameError> {
    match symbol.kind {
        SymbolKind::Skill => {
            if let Some(skill) = snapshot
                .skills
                .iter()
                .find(|skill| skill.plugin == document.plugin && skill.name == new_name)
            {
                return Err(RenameError::AlreadyExists {
                    path: skill.path.clone(),
                });
            }
        }
        SymbolKind::Agent => {
            if let Some(agent) = snapshot.agents.iter().find(|agent| agent.name == new_name) {
                return Err(RenameError::AlreadyExists {
                    path: agent.path.clone(),
                });
            }
        }
        SymbolKind::Fragment => {
            let scope = resolved_fragment(snapshot, document.plugin, &symbol.name)
                .and_then(|fragment| fragment.plugin.clone());
            let existing = snapshot
                .fragments
                .iter()
                .find(|fragment| fragment.name == new_name && fragment.plugin == scope);
            if let Some(fragment) = existing {
                return Err(RenameError::AlreadyExists {
                    path: fragment.path.clone(),
                });
            }
        }
        SymbolKind::Reference => {
            if let Some(skill) = snapshot.owner_skill(document.path) {
                if let Some(reference) = skill
                    .references
                    .iter()
                    .find(|reference| reference.name == new_name)
                {
                    return Err(RenameError::AlreadyExists {
                        path: reference.absolute_path.clone(),
                    });
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn file_renames(
    snapshot: &WorkspaceSnapshot,
    document: SourceDocument<'_>,
    symbol: &SymbolAtCursor,
    new_name: &str,
) -> Vec<FileRename> {
    match symbol.kind {
        SymbolKind::Skill => snapshot
            .skills
            .iter()
            .find(|skill| skill.plugin == document.plugin && skill.name == symbol.name)
            .map(|skill| {
                let old_dir = skill.path.parent().unwrap_or(&skill.path);
                let parent = old_dir.parent().unwrap_or(old_dir);
                vec![
                    FileRename {
                        old_path: skill.path.clone(),
                        new_path: old_dir.join(format!("{new_name}.pan")),
                    },
                    FileRename {
                        old_path: old_dir.to_path_buf(),
                        new_path: parent.join(new_name),
                    },
                ]
            })
            .unwrap_or_default(),
        SymbolKind::Reference => snapshot
            .owner_skill(document.path)
            .and_then(|skill| {
                skill
                    .references
                    .iter()
                    .find(|reference| reference.name == symbol.name)
            })
            .map(|reference| {
                let extension = reference
                    .absolute_path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("pan");
                let skill_dir = snapshot
                    .owner_skill(document.path)
                    .and_then(|skill| skill.path.parent())
                    .unwrap_or(Path::new(""));
                let new_path = skill_dir
                    .join("references")
                    .join(format!("{new_name}.{extension}"));
                vec![FileRename {
                    old_path: reference.absolute_path.clone(),
                    new_path,
                }]
            })
            .unwrap_or_default(),
        SymbolKind::Fragment => resolved_fragment(snapshot, document.plugin, &symbol.name)
            .map(|fragment| {
                let parent = fragment.path.parent().unwrap_or(&fragment.path);
                vec![FileRename {
                    old_path: fragment.path.clone(),
                    new_path: parent.join(format!("{new_name}.fragment.pan")),
                }]
            })
            .unwrap_or_default(),
        SymbolKind::Agent => snapshot
            .agents
            .iter()
            .find(|agent| agent.name == symbol.name)
            .map(|agent| {
                let parent = agent.path.parent().unwrap_or(&agent.path);
                vec![FileRename {
                    old_path: agent.path.clone(),
                    new_path: parent.join(format!("{new_name}.agent.pan")),
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(name: &str, relative_path: &str, extension: &str, root: &Path) -> Reference {
        let relative_path = format!("references/{relative_path}.{extension}");
        Reference {
            name: name.to_string(),
            absolute_path: root.join("src/core/owner").join(&relative_path),
            relative_path,
        }
    }

    fn snapshot() -> WorkspaceSnapshot {
        let root = PathBuf::from("/workspace");
        let skill = |name: &str, plugin: &str, source: &str| SkillDoc {
            name: name.to_string(),
            plugin: plugin.to_string(),
            src_dir: root.join(format!("src/{plugin}")),
            path: root.join(format!("src/{plugin}/{name}/{name}.pan")),
            source: source.to_string(),
            references: Vec::new(),
        };
        let mut owner = skill(
            "owner",
            "core",
            "---\nname: owner\ndescription: Owner\n---\n\n`skill::target` `ref::api/types` {> intro <} `agent::reviewer` `var::project` `env::CI` `cmd::cargo test` `url::https://example.com`\n",
        );
        owner.references = vec![
            reference("api/types", "api/types", "pan", &root),
            reference("notes", "notes", "md", &root),
        ];
        let cross_plugin = skill(
            "other",
            "extras",
            "---\nname: other\n---\n\n`skill::target` {> intro <}\n",
        );
        WorkspaceSnapshot {
            root: root.clone(),
            config_path: root.join("skillet.toml"),
            config_raw: "[vars]\nproject = \"skillet\"\n[env]\nCI = \"true\"\n".to_string(),
            skills: vec![
                owner,
                skill(
                    "target",
                    "core",
                    "---\nname: target\ndescription: Target\n---\n\nTarget body\n",
                ),
                skill(
                    "target",
                    "extras",
                    "---\nname: target\n---\n\nDifferent plugin\n",
                ),
                cross_plugin,
            ],
            agents: vec![AgentDoc {
                name: "reviewer".to_string(),
                plugin: "core".to_string(),
                path: root.join("agents/reviewer.agent.pan"),
                source: "---\nname: reviewer\ndescription: Review\n---\n\n`skill::target` `agent::reviewer`\n"
                    .to_string(),
            }],
            fragments: vec![
                FragmentDoc {
                    name: "intro".to_string(),
                    path: root.join("fragments/intro.fragment.pan"),
                    plugin: None,
                },
                FragmentDoc {
                    name: "intro".to_string(),
                    path: root.join("src/core/fragments/intro.fragment.pan"),
                    plugin: Some("core".to_string()),
                },
            ],
        }
    }

    fn offset(source: &str, needle: &str) -> usize {
        source.find(needle).unwrap() + needle.len() / 2
    }

    fn skill_source<'a>(snapshot: &'a WorkspaceSnapshot, name: &str) -> &'a SkillDoc {
        snapshot
            .skills
            .iter()
            .find(|skill| skill.name == name && skill.plugin == "core")
            .unwrap()
    }

    #[test]
    fn symbol_lookup_finds_each_ref_kind_and_fragment() {
        let snapshot = snapshot();
        let source = &snapshot.skills[0].source;
        let path = &snapshot.skills[0].path;
        for (needle, kind, name) in [
            ("skill::target", SymbolKind::Skill, "target"),
            ("ref::api/types", SymbolKind::Reference, "api/types"),
            ("{> intro <", SymbolKind::Fragment, "intro"),
            ("agent::reviewer", SymbolKind::Agent, "reviewer"),
            ("var::project", SymbolKind::Var, "project"),
            ("env::CI", SymbolKind::Env, "CI"),
            ("cmd::cargo test", SymbolKind::Cmd, "cargo test"),
            (
                "url::https://example.com",
                SymbolKind::Url,
                "https://example.com",
            ),
        ] {
            let found = symbol_at_offset(&snapshot, path, offset(source, needle)).unwrap();
            assert_eq!(found.kind, kind, "for {needle}");
            assert_eq!(found.name, name, "for {needle}");
            assert!(found.full_range.start <= found.value_range.start);
            assert!(found.value_range.end <= found.full_range.end);
            if needle == "skill::target" {
                let full_start = source.find("`skill::target`").unwrap();
                let value_start = source.find("target").unwrap();
                assert_eq!(
                    found.full_range,
                    ByteRange {
                        start: full_start,
                        end: full_start + "`skill::target`".len(),
                    }
                );
                assert_eq!(
                    found.value_range,
                    ByteRange {
                        start: value_start,
                        end: value_start + "target".len(),
                    }
                );
            }
        }
    }

    #[test]
    fn symbol_lookup_finds_frontmatter_and_ignores_escape_and_fenced_code() {
        let mut snapshot = snapshot();
        snapshot.skills[0].source.push_str(
            "\n``skill::target``\n`plainword`\n\n```pan\n`skill::target`\n```\n`skill::target`\n",
        );
        let path = snapshot.skills[0].path.clone();
        let source = snapshot.skills[0].source.clone();
        let fm = offset(&source, "name: owner") + "name: ".len() - "owner".len() / 2;
        assert_eq!(
            symbol_at_offset(&snapshot, &path, fm).unwrap().kind,
            SymbolKind::Skill
        );
        let escaped = source.find("``skill::target``").unwrap() + 5;
        let fenced = source.find("```pan\n`skill::target`").unwrap() + 10;
        let valid = source.rfind("`skill::target`").unwrap() + 8;
        let suspect = source.find("`plainword`").unwrap_or(0);
        assert!(symbol_at_offset(&snapshot, &path, escaped).is_none());
        assert!(symbol_at_offset(&snapshot, &path, fenced).is_none());
        assert!(symbol_at_offset(&snapshot, &path, suspect).is_none());
        assert_eq!(
            symbol_at_offset(&snapshot, &path, valid).unwrap().name,
            "target"
        );
    }

    #[test]
    fn definition_resolution_scopes_skills_and_resolves_refs_fragments_agents_and_config() {
        let snapshot = snapshot();
        let owner = skill_source(&snapshot, "owner");
        let source = &owner.source;
        let path = &owner.path;
        let skill_target =
            resolve_definition(&snapshot, path, offset(source, "skill::target")).unwrap();
        assert!(skill_target.path.ends_with("src/core/target/target.pan"));
        let ref_target =
            resolve_definition(&snapshot, path, offset(source, "ref::api/types")).unwrap();
        assert!(ref_target.path.ends_with("references/api/types.pan"));
        let fragment = resolve_definition(&snapshot, path, offset(source, "{> intro <}")).unwrap();
        assert!(fragment
            .path
            .ends_with("src/core/fragments/intro.fragment.pan"));
        let agent = resolve_definition(&snapshot, path, offset(source, "agent::reviewer")).unwrap();
        assert!(agent.path.ends_with("agents/reviewer.agent.pan"));
        for (needle, section, key) in [
            ("var::project", "[vars]", "project"),
            ("env::CI", "[env]", "CI"),
        ] {
            let target = resolve_definition(&snapshot, path, offset(source, needle)).unwrap();
            assert_eq!(target.path, snapshot.config_path);
            let range = target.target_range.unwrap();
            assert_eq!(
                &snapshot.config_raw[range.start..range.end],
                key,
                "in {section}"
            );
        }
        assert!(resolve_definition(&snapshot, path, offset(source, "cmd::cargo test")).is_none());
        assert!(
            resolve_definition(&snapshot, path, offset(source, "url::https://example.com"))
                .is_none()
        );

        let other = snapshot
            .skills
            .iter()
            .find(|skill| skill.name == "other")
            .unwrap();
        assert!(resolve_definition(
            &snapshot,
            &other.path,
            offset(&other.source, "skill::target")
        )
        .unwrap()
        .path
        .ends_with("src/extras/target/target.pan"));
        assert!(
            resolve_definition(&snapshot, &other.path, offset(&other.source, "{> intro <}"))
                .unwrap()
                .path
                .ends_with("fragments/intro.fragment.pan")
        );
    }

    #[test]
    fn reference_resolution_is_extension_agnostic_and_ambiguous_names_do_not_resolve() {
        let mut snapshot = snapshot();
        let owner = skill_source(&snapshot, "owner");
        let path = owner.path.clone();
        let source = owner.source.clone();
        snapshot.skills[0].references.push(reference(
            "api/types",
            "api/types",
            "md",
            &snapshot.root,
        ));
        assert!(resolve_definition(&snapshot, &path, offset(&source, "ref::api/types")).is_none());
        snapshot.skills[0].references.pop();
        snapshot.skills[0].references[0] =
            reference("api/types", "api/types", "md", &snapshot.root);
        assert!(
            resolve_definition(&snapshot, &path, offset(&source, "ref::api/types"))
                .unwrap()
                .path
                .ends_with("references/api/types.md")
        );
    }

    #[test]
    fn prepare_rename_accepts_resolved_symbols_and_rejects_nonrenameable_symbols() {
        let snapshot = snapshot();
        let owner = skill_source(&snapshot, "owner");
        for needle in [
            "skill::target",
            "ref::api/types",
            "{> intro <",
            "agent::reviewer",
        ] {
            assert!(prepare_rename(&snapshot, &owner.path, offset(&owner.source, needle)).is_ok());
        }
        assert!(prepare_rename(
            &snapshot,
            &owner.path,
            offset(&owner.source, "cmd::cargo test")
        )
        .is_err_and(|error| matches!(
            error,
            RenameError::NotRenameable {
                kind: SymbolKind::Cmd,
                ..
            }
        )));
        assert!(prepare_rename(
            &snapshot,
            &owner.path,
            offset(&owner.source, "url::https://example.com")
        )
        .is_err_and(|error| matches!(
            error,
            RenameError::NotRenameable {
                kind: SymbolKind::Url,
                ..
            }
        )));
        for needle in ["var::project", "env::CI"] {
            assert!(
                prepare_rename(&snapshot, &owner.path, offset(&owner.source, needle))
                    .is_err_and(|error| matches!(error, RenameError::NotRenameable { .. }))
            );
        }
        let skill = skill_source(&snapshot, "target");
        assert!(prepare_rename(
            &snapshot,
            &skill.path,
            offset(&skill.source, "name: target")
        )
        .is_ok());
        let agent = &snapshot.agents[0];
        assert!(prepare_rename(
            &snapshot,
            &agent.path,
            offset(&agent.source, "name: reviewer")
        )
        .is_ok());
        let mut unresolved = snapshot.clone();
        unresolved
            .skills
            .retain(|skill| !(skill.name == "target" && skill.plugin == "core"));
        let owner = skill_source(&unresolved, "owner");
        assert!(prepare_rename(
            &unresolved,
            &owner.path,
            offset(&owner.source, "skill::target")
        )
        .is_err_and(|error| matches!(error, RenameError::UnresolvedTarget { .. })));

        let mut unresolved_reference = snapshot.clone();
        unresolved_reference.skills[0].references.clear();
        let owner = skill_source(&unresolved_reference, "owner");
        assert!(prepare_rename(
            &unresolved_reference,
            &owner.path,
            offset(&owner.source, "ref::api/types")
        )
        .is_err_and(|error| matches!(error, RenameError::UnresolvedTarget { .. })));

        let mut external_reference = snapshot.clone();
        external_reference.skills[0].references[0].absolute_path =
            PathBuf::from("/external/types.pan");
        let owner = skill_source(&external_reference, "owner");
        assert!(prepare_rename(
            &external_reference,
            &owner.path,
            offset(&owner.source, "ref::api/types")
        )
        .is_err_and(|error| matches!(error, RenameError::NotRenameable { .. })));

        let mut external_skill = snapshot.clone();
        external_skill.skills[0].path = external_skill.root.join(".apm/core/owner/owner.pan");
        external_skill.skills[0].references[0].absolute_path = external_skill
            .root
            .join(".apm/core/owner/references/api/types.pan");
        let owner = skill_source(&external_skill, "owner");
        assert!(prepare_rename(
            &external_skill,
            &owner.path,
            offset(&owner.source, "ref::api/types")
        )
        .is_err_and(|error| matches!(error, RenameError::NotRenameable { .. })));
    }

    #[test]
    fn skill_rename_scopes_edits_and_orders_file_renames() {
        let snapshot = snapshot();
        let owner = skill_source(&snapshot, "owner");
        let plan = plan_rename(
            &snapshot,
            &owner.path,
            offset(&owner.source, "skill::target"),
            "new-target",
        )
        .unwrap();
        let paths = plan
            .text_edits
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        assert!(paths
            .iter()
            .any(|path| path.ends_with("src/core/owner/owner.pan")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("agents/reviewer.agent.pan")));
        assert!(!paths
            .iter()
            .any(|path| path.ends_with("src/extras/other/other.pan")));
        let target = skill_source(&snapshot, "target");
        let target_edits = plan
            .text_edits
            .iter()
            .find(|file| file.path == target.path)
            .unwrap();
        assert!(target_edits
            .edits
            .iter()
            .any(|edit| { &target.source[edit.range.start..edit.range.end] == "target" }));
        assert_eq!(plan.file_renames.len(), 2);
        assert!(plan.file_renames[0]
            .new_path
            .ends_with("target/new-target.pan"));
        assert!(plan.file_renames[1]
            .new_path
            .ends_with("src/core/new-target"));
    }

    #[test]
    fn reference_rename_supports_nested_names_preserves_extension_and_detects_collisions() {
        let mut snapshot = snapshot();
        snapshot.skills[0].references[0] =
            reference("api/types", "api/types", "md", &snapshot.root);
        let owner = skill_source(&snapshot, "owner");
        let owner_path = owner.path.clone();
        let owner_source = owner.source.clone();
        let plan = plan_rename(
            &snapshot,
            &owner_path,
            offset(&owner_source, "ref::api/types"),
            "api/models",
        )
        .unwrap();
        assert!(plan.file_renames[0]
            .new_path
            .ends_with("references/api/models.md"));
        let err = plan_rename(
            &snapshot,
            &owner_path,
            offset(&owner_source, "ref::api/types"),
            "../escape",
        )
        .unwrap_err();
        assert!(matches!(err, RenameError::InvalidName { .. }));
        let collision = plan_rename(
            &snapshot,
            &owner_path,
            offset(&owner_source, "ref::api/types"),
            "notes",
        )
        .unwrap_err();
        assert!(matches!(collision, RenameError::AlreadyExists { .. }));
        assert!(matches!(
            plan_rename(
                &snapshot,
                &owner_path,
                offset(&owner_source, "ref::api/types"),
                "api/models.md"
            ),
            Err(RenameError::InvalidName { .. })
        ));
        snapshot.skills[0].references.push(reference(
            "api/models",
            "api/models",
            "pan",
            &snapshot.root,
        ));
        assert!(matches!(
            plan_rename(
                &snapshot,
                &owner_path,
                offset(&owner_source, "ref::api/types"),
                "api/models"
            ),
            Err(RenameError::AlreadyExists { .. })
        ));
    }

    #[test]
    fn global_fragment_rename_skips_local_shadows_and_local_rename_stays_local() {
        let mut snapshot = snapshot();
        snapshot.skills[0].source = "---\nname: owner\n---\n{> global <}\n".to_string();
        let global_path = snapshot.fragments[0].path.clone();
        snapshot.fragments[0].name = "global".to_string();
        snapshot.fragments.push(FragmentDoc {
            name: "global".to_string(),
            path: PathBuf::from("/workspace/src/extras/fragments/global.fragment.pan"),
            plugin: Some("extras".to_string()),
        });
        snapshot.skills[3].source = "---\nname: other\n---\n{> global <}\n".to_string();
        let owner = skill_source(&snapshot, "owner");
        let global = plan_rename(
            &snapshot,
            &owner.path,
            offset(&owner.source, "{> global <}"),
            "renamed",
        )
        .unwrap();
        assert!(global.text_edits.iter().any(|file| file.path == owner.path));
        assert!(!global
            .text_edits
            .iter()
            .any(|file| file.path.ends_with("other/other.pan")));
        assert_eq!(global.file_renames[0].old_path, global_path);

        let local_path = snapshot.fragments[1].path.clone();
        snapshot.skills[0].source = "---\nname: owner\n---\n{> intro <}\n".to_string();
        let owner = skill_source(&snapshot, "owner");
        let local = plan_rename(
            &snapshot,
            &owner.path,
            offset(&owner.source, "{> intro <}"),
            "welcome",
        )
        .unwrap();
        assert_eq!(local.file_renames[0].old_path, local_path);
        assert!(local.file_renames[0]
            .new_path
            .ends_with("welcome.fragment.pan"));
    }

    #[test]
    fn agent_rename_updates_workspace_refs_frontmatter_and_filename() {
        let snapshot = snapshot();
        let agent = &snapshot.agents[0];
        let plan = plan_rename(
            &snapshot,
            &agent.path,
            offset(&agent.source, "name: reviewer") + "name: ".len(),
            "auditor",
        )
        .unwrap();
        assert!(plan.text_edits.iter().any(|file| file.path == agent.path));
        assert!(plan.text_edits.iter().any(|file| {
            file.path.ends_with("owner/owner.pan")
                && file.edits.iter().any(|edit| edit.new_text == "auditor")
        }));
        assert!(plan.file_renames[0]
            .new_path
            .ends_with("agents/auditor.agent.pan"));
    }

    #[test]
    fn agent_and_fragment_renames_reject_in_scope_collisions() {
        let mut snapshot = snapshot();
        snapshot.agents.push(AgentDoc {
            name: "auditor".to_string(),
            plugin: "extras".to_string(),
            path: snapshot.root.join("agents/auditor.agent.pan"),
            source: "---\nname: auditor\n---\n".to_string(),
        });
        let agent = &snapshot.agents[0];
        assert!(matches!(
            plan_rename(
                &snapshot,
                &agent.path,
                offset(&agent.source, "name: reviewer"),
                "auditor"
            ),
            Err(RenameError::AlreadyExists { .. })
        ));

        snapshot.fragments.push(FragmentDoc {
            name: "welcome".to_string(),
            path: snapshot
                .root
                .join("src/core/fragments/welcome.fragment.pan"),
            plugin: Some("core".to_string()),
        });
        let owner = skill_source(&snapshot, "owner");
        assert!(matches!(
            plan_rename(
                &snapshot,
                &owner.path,
                offset(&owner.source, "{> intro <}"),
                "welcome"
            ),
            Err(RenameError::AlreadyExists { .. })
        ));
    }

    #[test]
    fn rename_validation_rejects_invalid_names_noops_and_target_collisions() {
        let snapshot = snapshot();
        let owner = skill_source(&snapshot, "owner");
        for invalid in ["", "-bad", "Bad", "has--double"] {
            assert!(matches!(
                plan_rename(
                    &snapshot,
                    &owner.path,
                    offset(&owner.source, "skill::target"),
                    invalid
                ),
                Err(RenameError::InvalidName { .. })
            ));
        }
        assert!(matches!(
            plan_rename(
                &snapshot,
                &owner.path,
                offset(&owner.source, "skill::target"),
                "target"
            ),
            Err(RenameError::NoOp)
        ));
        assert!(matches!(
            plan_rename(
                &snapshot,
                &owner.path,
                offset(&owner.source, "skill::target"),
                "owner"
            ),
            Err(RenameError::AlreadyExists { .. })
        ));
    }
}
