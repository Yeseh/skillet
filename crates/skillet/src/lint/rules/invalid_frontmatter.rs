//! Rule: `invalid-frontmatter` — verifies `name` matches directory and `description` is present.

use crate::frontmatter::parse_frontmatter;

use super::{diag, Diagnostic, Severity};

/// Checks frontmatter validity for a skill's raw source.
pub fn check(name: &str, raw: &str) -> Vec<Diagnostic> {
    let fm = match parse_frontmatter(raw) {
        Err(e) => {
            return vec![diag(
                Severity::Error,
                name,
                "invalid-frontmatter",
                format!("failed to parse frontmatter: {e}"),
            )]
        }
        Ok(None) => {
            return vec![diag(
                Severity::Error,
                name,
                "invalid-frontmatter",
                "missing frontmatter".into(),
            )]
        }
        Ok(Some(fm)) => fm,
    };

    let mut diags = Vec::new();

    match fm.name.as_deref() {
        None => diags.push(diag(
            Severity::Error,
            name,
            "invalid-frontmatter",
            "missing 'name' field".into(),
        )),
        Some(n) if n != name => diags.push(diag(
            Severity::Error,
            name,
            "invalid-frontmatter",
            format!("name '{}' does not match directory '{}'", n, name),
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
            name,
            "invalid-frontmatter",
            "missing or empty 'description' field".into(),
        ));
    }

    diags
}
