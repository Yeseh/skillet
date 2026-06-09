//! Stage 4 — AST-based compiler for `.pan` body text.
//!
//! Pure text transformation: fragment expansion, var/env substitution,
//! backtick-wrapping of typed refs.  Assumes a prior [`crate::check`] pass
//! has validated referential integrity; unresolvable refs are silently
//! substituted with an empty string or passed through verbatim.

use std::collections::HashMap;

use crate::workspace::Workspace;

use super::{
    parse::{Node, PanParse, RefKind},
    PanSource,
};

// ── Public types ───────────────────────────────────────────────────────────────

/// Output of a single [`compile`] call.
pub struct CompileOutput {
    /// Compiled text: frontmatter section prepended to the compiled body.
    pub text: String,
    /// Fragment ids expanded, in first-use order (deduplicated).
    pub fragments_used: Vec<String>,
    /// Tiktoken count over `text`.
    pub activation_tokens: u32,
    /// Token count over `"{name} {description}"` from the frontmatter.
    pub discovery_tokens: u32,
}

/// Fragment content pre-rendered for an entire workspace.
///
/// Build once with [`render_fragments`] before the per-file compilation loop.
#[derive(Debug, Clone)]
pub struct RenderedFragments {
    /// Fragment id → trimmed content, ready to interpolate.
    pub rendered: HashMap<String, String>,
    /// Fragment id → reason string for fragments that cannot be expanded.
    pub poisoned: HashMap<String, String>,
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Compiles a single `.pan` source to its output text.
///
/// Infallible: assumes [`crate::check::check`] has already run and returned
/// no errors.  Unresolvable refs fall back gracefully (empty string for
/// var/env, verbatim backtick for structural refs).
pub fn compile(ws: &Workspace, source: &PanSource) -> CompileOutput {
    let raw = source.as_str();
    let body_offset = body_start_offset(raw);
    let body = &raw[body_offset..];
    let fm_section = &raw[..body_offset];

    let body_source = PanSource::new(body.to_string());
    let mut parser = PanParse::new(&body_source);
    parser.parse();

    let mut out = String::with_capacity(body.len());
    let mut fragments_used: Vec<String> = Vec::new();

    for node in &parser.nodes {
        match node {
            Node::Body { source_range } => {
                out.push_str(&body[source_range.start as usize..source_range.end as usize]);
            }
            Node::EscapedBody { source_range } => {
                let raw_slice = &body[source_range.start as usize..source_range.end as usize];
                out.push('`');
                out.push_str(&raw_slice[2..raw_slice.len() - 2]);
                out.push('`');
            }
            Node::RefSuspect { source_range } => {
                out.push_str(&body[source_range.start as usize..source_range.end as usize]);
            }
            Node::MarkdownLink { source_range, .. } => {
                out.push_str(&body[source_range.start as usize..source_range.end as usize]);
            }
            Node::Fragment { value, .. } => {
                let id = value.trim();
                if let Some(content) = ws.fragments.rendered.get(id) {
                    out.push_str(content);
                    if !fragments_used.contains(&id.to_string()) {
                        fragments_used.push(id.to_string());
                    }
                }
            }
            Node::Ref { kind, value, .. } => match kind {
                RefKind::Var => {
                    if let Some(v) = ws.vars.get(value.as_str()) {
                        out.push_str(v);
                    }
                }
                RefKind::Env => {
                    let resolved = std::env::var(value.as_str()).unwrap_or_else(|_| {
                        ws.env
                            .get(value.as_str())
                            .map(|e| e.default.clone())
                            .unwrap_or_default()
                    });
                    out.push_str(&resolved);
                }
                _ => {
                    out.push('`');
                    out.push_str(value);
                    out.push('`');
                }
            },
        }
    }

    let full_text = format!("{}{}", fm_section, out);
    let activation_tokens = crate::tokens::count_tokens(&full_text, &ws.tokenizer);

    let discovery_tokens = {
        let fm = crate::frontmatter::parse_frontmatter(source.as_str())
            .ok()
            .flatten();
        let text = fm
            .map(|f| {
                format!(
                    "{} {}",
                    f.name.unwrap_or_default(),
                    f.description.unwrap_or_default()
                )
            })
            .unwrap_or_default();
        crate::tokens::count_tokens(&text, &ws.tokenizer)
    };

    CompileOutput {
        text: full_text,
        fragments_used,
        activation_tokens,
        discovery_tokens,
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────────

pub(crate) fn body_start_offset(raw: &str) -> usize {
    if !raw.starts_with("---") {
        return 0;
    }
    let rest = &raw[3..];
    if let Some(close) = rest.find("\n---") {
        let after_dash = &rest[close + 4..];
        let skip = after_dash
            .find('\n')
            .map(|i| i + 1)
            .unwrap_or(after_dash.len());
        3 + close + 4 + skip
    } else {
        0
    }
}

/// Pre-renders all fragments for an entire workspace.
///
/// Trailing newlines are stripped from rendered content.  Fragments
/// containing nested `{> ... <}` directives or typed refs are placed in
/// `poisoned`.
pub fn render_fragments(raw: &HashMap<String, String>) -> RenderedFragments {
    let mut rendered: HashMap<String, String> = HashMap::with_capacity(raw.len());
    let mut poisoned: HashMap<String, String> = HashMap::new();

    for (id, content) in raw {
        let frag_src = PanSource::new(content.clone());
        let mut parser = PanParse::new(&frag_src);
        parser.parse();

        let nested = parser
            .nodes
            .iter()
            .find(|n| matches!(n, Node::Fragment { .. }));

        let has_ref = parser.nodes.iter().find(|n| matches!(n, Node::Ref { .. }));

        if let Some(Node::Fragment {
            value: nested_id, ..
        }) = nested
        {
            poisoned.insert(
                id.clone(),
                format!(
                    "nested include detected (it includes '{}') — nesting is not supported",
                    nested_id.trim()
                ),
            );
        } else if let Some(Node::Ref { kind, value, .. }) = has_ref {
            let prefix = match kind {
                RefKind::Agent => "agent",
                RefKind::Skill => "skill",
                RefKind::Cmd => "cmd",
                RefKind::Path => "path",
                RefKind::Url => "url",
                RefKind::Reference => "ref",
                RefKind::Var => "var",
                RefKind::Env => "env",
            };
            poisoned.insert(
                id.clone(),
                format!(
                    "ref detected ('{}::{}') — refs are not supported in fragments",
                    prefix, value
                ),
            );
        } else {
            rendered.insert(id.clone(), content.trim_end_matches('\n').to_string());
        }
    }

    RenderedFragments { rendered, poisoned }
}
