//! Rule: `unused-fragment` — warns when a fragment is not included by any skill.

use crate::workspace::Workspace;
use std::collections::HashSet;

use super::{diag, CompiledSkill, Diagnostic, Severity};

/// Finds fragments that no skill expands.
///
/// Usage is read from each skill's `fragments_used` (produced by the compile
/// stage), so it tracks exactly what the compiler actually expanded.
pub fn check(compiled: &[CompiledSkill], ws: &Workspace) -> Vec<Diagnostic> {
    let fragment_names = ws.fragment_names();
    if fragment_names.is_empty() {
        return vec![];
    }

    let used: HashSet<&str> = compiled
        .iter()
        .flat_map(|cs| cs.output.fragments_used.iter().map(|s| s.as_str()))
        .collect();

    fragment_names
        .into_iter()
        .filter(|frag_name| !used.contains(*frag_name))
        .map(|frag_name| {
            diag(
                Severity::Warning,
                "<workspace>",
                "unused-fragment",
                format!("fragment '{frag_name}' is not included by any skill"),
            )
        })
        .collect()
}
