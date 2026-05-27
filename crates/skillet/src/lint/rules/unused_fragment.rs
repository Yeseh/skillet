//! Rule: `unused-fragment` — warns when a fragment is not included by any skill.

use crate::config::SkilletConfig;
use crate::lint::pipeline::SourceFile;
use crate::lint::LintContext;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::{diag, Diagnostic, Severity};

static FRAGMENT_INCLUDE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\{\{>\s*([\w-]+)\s*\}\}").unwrap());

/// Finds unused fragments using the pre-read `raw` content from Phase 1.
pub fn check(
    source_files: &[SourceFile],
    ctx: &LintContext,
    _config: &SkilletConfig,
) -> Vec<Diagnostic> {
    if ctx.fragment_names.is_empty() {
        return vec![];
    }

    let mut used: HashSet<String> = HashSet::new();
    for sf in source_files {
        for caps in FRAGMENT_INCLUDE_RE.captures_iter(&sf.raw) {
            used.insert(caps[1].to_string());
        }
    }

    ctx.fragment_names
        .iter()
        .filter(|frag_name| !used.contains(frag_name.as_str()))
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
