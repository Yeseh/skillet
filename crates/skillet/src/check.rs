//! Referential integrity checking for `.pan` source files.
//!
//! Runs before compilation in both the build and lint pipelines.  Reports
//! broken fragment references, undeclared vars/env, missing ref paths, and
//! commands absent from PATH.  Does not write any files.

use std::collections::HashSet;

use crate::compiler::{
    PanSource,
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

/// A diagnostic produced by [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDiag {
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
pub fn check(ws: &Workspace, source: &PanSource, known_files: &HashSet<String>) -> Vec<CheckDiag> {
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
                        severity: Severity::Error,
                        message: format!("cannot expand fragment '{}': {}", id, reason),
                        line,
                        col,
                    });
                } else if !ws.fragments.rendered.contains_key(id) {
                    diags.push(CheckDiag {
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
                                severity: Severity::Error,
                                message: format!("ref path not found: '{}'", value),
                                line,
                                col,
                            });
                        }
                    }
                    RefKind::Cmd => {
                        let cmd = value.split_whitespace().next().unwrap_or(value.as_str());
                        if !workspace::is_on_path(cmd) {
                            diags.push(CheckDiag {
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

fn body_start_offset(raw: &str) -> usize {
    if !raw.starts_with("---") {
        return 0;
    }
    let rest = &raw[3..];
    if let Some(close) = rest.find("\n---") {
        let after_dash = &rest[close + 4..];
        let skip = after_dash.find('\n').map(|i| i + 1).unwrap_or(after_dash.len());
        3 + close + 4 + skip
    } else {
        0
    }
}
