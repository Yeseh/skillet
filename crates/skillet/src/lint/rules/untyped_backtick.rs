//! Rule: `untyped-backtick` — nudges authors toward explicit ref annotations.

use crate::lint::pipeline::SourceFile;

use super::{diag_located, Diagnostic, Severity};

/// Emits info diagnostics for untyped backticks using pre-extracted data from Phase 2.
pub fn check(source: &SourceFile) -> Vec<Diagnostic> {
    let file_path = source.source_path.to_string_lossy().to_string();

    source
        .parsed_refs
        .untyped
        .iter()
        .map(|u| {
            diag_located(
                Severity::Info,
                &source.name,
                "untyped-backtick",
                format!(
                    "`{}` looks like a {} — consider `{}::{}`",
                    u.content, u.inferred_kind, u.inferred_kind, u.content
                ),
                Some(file_path.clone()),
                Some(u.line),
                Some(u.col),
            )
        })
        .collect()
}
