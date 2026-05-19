//! Rule: `untyped-backtick` — nudges authors toward explicit ref annotations.

use crate::refs::ParsedRefs;
use crate::workspace::SkillSource;

use super::{diag_located, Diagnostic, Severity};

pub fn check(source: &SkillSource, parsed: &ParsedRefs) -> Vec<Diagnostic> {
    let file_path = source.source_path.to_string_lossy().to_string();

    parsed
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
