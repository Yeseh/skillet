//! Rule: `stale-build` — verifies the compiled `SKILL.md` matches the source.

use crate::lint::pipeline::SourceFile;
use crate::lint::LintContext;
use crate::lockfile::Lockfile;

use super::{diag, Diagnostic, Severity};

/// Checks staleness using the pre-computed source hash from Phase 1 and
/// pre-loaded hashes from `LintContext`.
pub fn check(source: &SourceFile, lockfile: &Lockfile, ctx: &LintContext) -> Vec<Diagnostic> {
    if !ctx.compiled_hashes.contains_key(&source.name) {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md not found — run `skillet build`".into(),
        )];
    }

    let Some(entry) = lockfile.skills.get(&source.name) else {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md not in lockfile — run `skillet build`".into(),
        )];
    };

    // Use the pre-computed hash from Phase 1 — no re-hashing.
    if !source.source_hash.is_empty() && source.source_hash != entry.source_hash {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md is out of date — run `skillet build`".into(),
        )];
    }

    for frag_name in &entry.fragments_used {
        let current = match ctx.fragment_hashes.get(frag_name.as_str()) {
            Some(h) => h.as_str(),
            None => {
                return vec![diag(
                    Severity::Error,
                    &source.name,
                    "stale-build",
                    format!("cannot hash fragment '{frag_name}': not found"),
                )]
            }
        };
        let locked = lockfile
            .fragments
            .get(frag_name)
            .map(|f| f.hash.as_str())
            .unwrap_or("");
        if current != locked {
            return vec![diag(
                Severity::Error,
                &source.name,
                "stale-build",
                "SKILL.md is out of date — run `skillet build`".into(),
            )];
        }
    }

    let compiled_hash = ctx.compiled_hashes.get(&source.name).unwrap();
    if compiled_hash != &entry.compiled_hash {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md is out of date — run `skillet build`".into(),
        )];
    }

    vec![]
}
