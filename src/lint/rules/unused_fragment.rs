//! Rule: `unused-fragment` — warns when a fragment is not included by any skill.

use crate::config::SkilletConfig;
use crate::workspace::SkillSource;
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use super::{diag, Diagnostic, Severity};

static FRAGMENT_INCLUDE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\{\{>\s*([\w-]+)\s*\}\}").unwrap());

pub fn check(
    all_sources: &[SkillSource],
    fragments_dir: &Path,
    _config: &SkilletConfig,
) -> Vec<Diagnostic> {
    if !fragments_dir.exists() {
        return vec![];
    }

    let mut used: HashSet<String> = HashSet::new();
    for source in all_sources {
        if let Ok(raw) = std::fs::read_to_string(&source.source_path) {
            for caps in FRAGMENT_INCLUDE_RE.captures_iter(&raw) {
                used.insert(caps[1].to_string());
            }
        }
    }

    let Ok(entries) = std::fs::read_dir(fragments_dir) else {
        return vec![];
    };

    entries
        .flatten()
        .filter_map(|e| {
            let fname = e.file_name().into_string().ok()?;
            let frag_name = fname.strip_suffix(".fragment.pan")?.to_string();
            if used.contains(&frag_name) {
                return None;
            }
            Some(diag(
                Severity::Warning,
                "<workspace>",
                "unused-fragment",
                format!("fragment '{frag_name}' is not included by any skill"),
            ))
        })
        .collect()
}
