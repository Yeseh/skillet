//! Stage 4 — AST-based compiler for `.pan` body text.
//!
//! Design decisions locked in before this was written:
//! - **Plain `match`** over AST nodes — no Visitor trait.
//! - **Pre-pass** renders fragments once per workspace into a [`RenderedFragments`]
//!   struct before the per-file compilation loop begins.
//! - **Main pass** interpolates the pre-rendered strings and resolves refs
//!   inline while walking the node list once.
//! - **Single tiktoken pass** over the fully assembled output string at the end.

use std::collections::{HashMap};
use crate::workspace::Workspace;

use super::{
    parse::{Node, PanParse, RefKind},
    PanSource,
};

// ── Public types ───────────────────────────────────────────────────────────────



/// Fragment content pre-rendered for an entire workspace.
///
/// Build this **once** with [`render_fragments`] before starting the
/// per-file compilation loop.  Pass a reference to every [`compile_body`]
/// call so that fragment parsing is never repeated across files.
#[derive(Debug, Clone)]
pub struct RenderedFragments {
    /// Fragment id → trimmed content, ready to interpolate.
    pub rendered: HashMap<String, String>,
    /// Fragment id → reason string for fragments that cannot be expanded
    /// (e.g. they contain a nested include directive).
    pub poisoned: HashMap<String, String>,
}

// ── Public API ─────────────────────────────────────────────────────────────────


pub fn compile(ws: &Workspace, source: &PanSource)  {
    
}

// ── Internal helpers ───────────────────────────────────────────────────────────

/// Pre-renders all fragments for an entire workspace.
///
/// Call this **once** before compiling any files, then pass the returned
/// [`RenderedFragments`] to every [`compile_body`] call.  Each fragment's
/// content is parsed to detect nested `{> ... <}` directives; fragments that
/// contain nesting go into `poisoned` instead of `rendered` so that every
/// skill file that tries to use them gets a clear, located error rather than
/// a generic "not found".
///
/// Trailing newlines are stripped from rendered content.
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

        match nested {
            Some(Node::Fragment {
                value: nested_id, ..
            }) => {
                poisoned.insert(
                    id.clone(),
                    format!(
                        "nested include detected (it includes '{}') — nesting is not supported",
                        nested_id.trim()
                    ),
                );
            }
            _ => {
                rendered.insert(id.clone(), content.trim_end_matches('\n').to_string());
            }
        }
    }

    RenderedFragments { rendered, poisoned }
}

