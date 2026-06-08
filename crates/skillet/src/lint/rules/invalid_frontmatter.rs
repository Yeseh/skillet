//! Rule: `invalid-frontmatter` — verifies `name` matches directory and `description` is present.

use crate::config::SkilletConfig;
use crate::lint::pipeline::SourceFile;

use super::{diag, Diagnostic, Severity};

/// Checks frontmatter validity using the pre-parsed data from Phase 1.
pub fn check(source: &SourceFile, _config: &SkilletConfig) -> Vec<Diagnostic> {
    // Surface Phase 1 parse errors first.
    if !source.parse_errors.is_empty() {
        return source
            .parse_errors
            .iter()
            .map(|e| {
                diag(
                    Severity::Error,
                    &source.name,
                    "invalid-frontmatter",
                    format!("failed to parse frontmatter: {e}"),
                )
            })
            .collect();
    }

    let fm = match source.frontmatter.as_ref() {
        None => {
            return vec![diag(
                Severity::Error,
                &source.name,
                "invalid-frontmatter",
                "missing frontmatter".into(),
            )]
        }
        Some(fm) => fm,
    };

    let mut diags = Vec::new();

    match fm.name.as_deref() {
        None => diags.push(diag(
            Severity::Error,
            &source.name,
            "invalid-frontmatter",
            "missing 'name' field".into(),
        )),
        Some(n) if n != source.name => diags.push(diag(
            Severity::Error,
            &source.name,
            "invalid-frontmatter",
            format!("name '{}' does not match directory '{}'", n, source.name),
        )),
        _ => {}
    }

    if fm
        .description
        .as_deref()
        .map(|d: &str| d.trim().is_empty())
        .unwrap_or(true)
    {
        diags.push(diag(
            Severity::Error,
            &source.name,
            "invalid-frontmatter",
            "missing or empty 'description' field".into(),
        ));
    }

    diags
}
