//! Rule: `untyped-backtick` — nudges authors toward explicit ref annotations.

use crate::config::SkilletConfig;
use crate::refs::{classify_untyped, extract_body, TYPED_REF_RE, UNTYPED_BACKTICK_RE};
use crate::workspace::SkillSource;

use super::{diag, Diagnostic, Severity};

pub fn check(
    source: &SkillSource,
    raw: &str,
    all_sources: &[SkillSource],
    _config: &SkilletConfig,
) -> Vec<Diagnostic> {
    let body = extract_body(raw);
    let stripped = TYPED_REF_RE.replace_all(&body, "");

    let skill_names: Vec<&str> = all_sources.iter().map(|s| s.name.as_str()).collect();
    let mut diags = Vec::new();

    for caps in UNTYPED_BACKTICK_RE.captures_iter(&stripped) {
        let content = caps[1].trim();
        if let Some(kind) = classify_untyped(content, &skill_names) {
            diags.push(diag(
                Severity::Info,
                &source.name,
                "untyped-backtick",
                format!("`{content}` looks like a {kind} — consider `{kind}::{content}`"),
            ));
        }
    }

    diags
}
