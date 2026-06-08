//! Rule: `untyped-backtick` — nudges authors toward explicit ref annotations.

use crate::refs::ParsedRefs;

use super::{diag_located, Diagnostic, Severity};

/// Emits info diagnostics for untyped backticks that look like refs.
///
/// This is a lint-only heuristic (the compiler treats untyped backticks as
/// literal text), so it extracts the untyped refs itself via [`ParsedRefs`].
/// `skill_names` lets the classifier recognise cross-skill references.
pub fn check(name: &str, file_path: &str, raw: &str, skill_names: &[&str]) -> Vec<Diagnostic> {
    let parsed = ParsedRefs::extract(raw, skill_names);

    parsed
        .untyped
        .iter()
        .map(|u| {
            diag_located(
                Severity::Info,
                name,
                "untyped-backtick",
                format!(
                    "`{}` looks like a {} — consider `{}::{}`",
                    u.content, u.inferred_kind, u.inferred_kind, u.content
                ),
                Some(file_path.to_string()),
                Some(u.line),
                Some(u.col),
            )
        })
        .collect()
}
