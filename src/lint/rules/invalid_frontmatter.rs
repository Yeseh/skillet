//! Rule: `invalid-frontmatter` — verifies `name` matches directory and `description` is present.

use crate::config::SkilletConfig;
use crate::parse::parse_frontmatter;
use crate::workspace::SkillSource;

use super::{diag, Diagnostic, Severity};

pub fn check(source: &SkillSource, raw: &str, _config: &SkilletConfig) -> Vec<Diagnostic> {
    let fm = match parse_frontmatter(raw) {
        Err(e) => {
            return vec![diag(
                Severity::Error,
                &source.name,
                "invalid-frontmatter",
                format!("failed to parse frontmatter: {e}"),
            )]
        }
        Ok(None) => {
            return vec![diag(
                Severity::Error,
                &source.name,
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

    if fm.description.as_deref().map(|d| d.trim().is_empty()).unwrap_or(true) {
        diags.push(diag(
            Severity::Error,
            &source.name,
            "invalid-frontmatter",
            "missing or empty 'description' field".into(),
        ));
    }

    diags
}
