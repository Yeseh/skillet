//! Referential integrity checking for `.pan` source files.
//!
//! Runs before compilation in both the build and lint pipelines.  Reports
//! broken fragment references, undeclared vars/env, missing ref paths, and
//! commands absent from PATH.  Does not write any files.

use std::collections::HashSet;

use crate::compiler::{
    PanSource,
    compile::body_start_offset,
    parse::{Node, PanParse, RefKind},
};
use crate::workspace::{self, Workspace};

/// Severity of a check diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard error; checking fails.
    Error,
    /// A non-fatal advisory.
    Warning,
}

/// The category of referential-integrity problem a [`CheckDiag`] describes.
///
/// Callers (e.g. the lint engine) use this to attach a stable rule identifier
/// instead of pattern-matching on the human-readable message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// A `{> fragment <}` include that is missing or cannot be expanded.
    Fragment,
    /// A `ref::` path that does not exist in the skill directory.
    PathRef,
    /// A `cmd::` command not found on PATH.
    Command,
    /// A `skill::` reference to an unknown skill.
    Skill,
    /// A `var::` reference not declared in `[vars]`.
    Var,
    /// An `env::` reference not declared in `[env]`.
    Env,
}

/// A diagnostic produced by [`check_source_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDiag {
    /// Category of the problem.
    pub kind: CheckKind,
    /// Severity of the diagnostic.
    pub severity: Severity,
    /// Human-readable description of the problem.
    pub message: String,
    /// 1-based line in the full source file (frontmatter included).
    pub line: u32,
    /// 1-based column.
    pub col: u32,
}

/// Checks referential integrity of a single `.pan` source file.
///
/// `known_files` is the set of relative paths under the artifact's source
/// directory used to validate `ref::` targets.  Pass an empty set to skip
/// that check (e.g. for agents).
pub fn check_source_file(ws: &Workspace, source: &PanSource, known_files: &HashSet<String>) -> Vec<CheckDiag> {
    let raw = source.as_str();
    let body_offset = body_start_offset(raw);
    let body = &raw[body_offset..];
    let fm_line_offset = raw[..body_offset].matches('\n').count() as u32;

    let body_source = PanSource::new(body.to_string());
    let mut parser = PanParse::new(&body_source);
    parser.parse();

    let mut diags: Vec<CheckDiag> = Vec::new();

    for node in &parser.nodes {
        match node {
            Node::Fragment { value, source_range } => {
                let id = value.trim();
                let loc = body_source.location_at(source_range.start);
                let line = loc.line + fm_line_offset;
                let col = loc.column;
                if let Some(reason) = ws.fragments.poisoned.get(id) {
                    diags.push(CheckDiag {
                        kind: CheckKind::Fragment,
                        severity: Severity::Error,
                        message: format!("cannot expand fragment '{}': {}", id, reason),
                        line,
                        col,
                    });
                } else if !ws.fragments.rendered.contains_key(id) {
                    diags.push(CheckDiag {
                        kind: CheckKind::Fragment,
                        severity: Severity::Error,
                        message: format!("fragment '{}' not found", id),
                        line,
                        col,
                    });
                }
            }
            Node::Ref { kind, value, source_range } => {
                let loc = body_source.location_at(source_range.start);
                let line = loc.line + fm_line_offset;
                let col = loc.column;
                match kind {
                    RefKind::Reference => {
                        if !known_files.is_empty() && !known_files.contains(value.as_str()) {
                            diags.push(CheckDiag {
                                kind: CheckKind::PathRef,
                                severity: Severity::Error,
                                message: format!("ref path not found: '{}'", value),
                                line,
                                col,
                            });
                        }
                    }
                    RefKind::Cmd => {
                        let cmd = value.split_whitespace().next().unwrap_or(value.as_str());
                        if !ws.allowed_commands.contains(cmd) && !workspace::is_on_path(cmd) {
                            diags.push(CheckDiag {
                                kind: CheckKind::Command,
                                severity: Severity::Warning,
                                message: format!("command '{}' not found on PATH", cmd),
                                line,
                                col,
                            });
                        }
                    }
                    RefKind::Skill => {
                        if !ws.skills.is_empty() && !ws.skills.contains_key(value.as_str()) {
                            diags.push(CheckDiag {
                                kind: CheckKind::Skill,
                                severity: Severity::Error,
                                message: format!("skill '{}' not found in workspace", value),
                                line,
                                col,
                            });
                        }
                    }
                    RefKind::Var => {
                        if !ws.vars.contains_key(value.as_str()) {
                            diags.push(CheckDiag {
                                kind: CheckKind::Var,
                                severity: Severity::Error,
                                message: format!("var '{}' not declared in [vars]", value),
                                line,
                                col,
                            });
                        }
                    }
                    RefKind::Env => {
                        if !ws.env.contains_key(value.as_str()) {
                            diags.push(CheckDiag {
                                kind: CheckKind::Env,
                                severity: Severity::Error,
                                message: format!("env '{}' not declared in [env]", value),
                                line,
                                col,
                            });
                        }
                    }
                    RefKind::Agent | RefKind::Path | RefKind::Url => {}
                }
            }
            _ => {}
        }
    }

    diags
}

