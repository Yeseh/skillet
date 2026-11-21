//! Rule: `invalid-frontmatter` — verifies `name` matches directory and `description` is present.

use crate::frontmatter::parse_frontmatter;

use super::{diag_located, frontmatter_field_line, Diagnostic, Severity};

/// Checks frontmatter validity for a skill's raw source.
pub fn check(name: &str, file_path: &str, raw: &str) -> Vec<Diagnostic> {
    let path = Some(file_path.to_string());
    let fm = match parse_frontmatter(raw) {
        Err(e) => {
            return vec![diag_located(
                Severity::Error,
                name,
                "invalid-frontmatter",
                format!("failed to parse frontmatter: {e}"),
                path,
                Some(1),
                None,
            )]
        }
        Ok(None) => {
            return vec![diag_located(
                Severity::Error,
                name,
                "invalid-frontmatter",
                "missing frontmatter".into(),
                path,
                Some(1),
                None,
            )]
        }
        Ok(Some(fm)) => fm,
    };

    let mut diags = Vec::new();

    match fm.name.as_deref() {
        None => diags.push(diag_located(
            Severity::Error,
            name,
            "invalid-frontmatter",
            "missing 'name' field".into(),
            path.clone(),
            Some(1),
            None,
        )),
        Some(n) if n != name => diags.push(diag_located(
            Severity::Error,
            name,
            "invalid-frontmatter",
            format!("name '{}' does not match directory '{}'", n, name),
            path.clone(),
            frontmatter_field_line(raw, "name").or(Some(1)),
            None,
        )),
        _ => {}
    }

    if fm
        .description
        .as_deref()
        .map(|d: &str| d.trim().is_empty())
        .unwrap_or(true)
    {
        diags.push(diag_located(
            Severity::Error,
            name,
            "invalid-frontmatter",
            "missing or empty 'description' field".into(),
            path,
            frontmatter_field_line(raw, "description").or(Some(1)),
            None,
        ));
    }

    diags
}
