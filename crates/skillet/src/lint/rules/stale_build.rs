//! Rule: `stale-build` — verifies the compiled `SKILL.md` matches the source.

use crate::lockfile::Lockfile;
use crate::workspace::{hash_file, SkillSource};
use std::path::Path;

use super::{diag, Diagnostic, Severity};

pub fn check(source: &SkillSource, fragments_dir: &Path, lockfile: &Lockfile) -> Vec<Diagnostic> {
    let output_path = source.skill_out_dir.join("SKILL.md");

    if !output_path.exists() {
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

    let source_hash = match hash_file(&source.source_path) {
        Ok(h) => h,
        Err(e) => {
            return vec![diag(
                Severity::Error,
                &source.name,
                "stale-build",
                format!("cannot hash source: {e}"),
            )]
        }
    };

    if source_hash != entry.source_hash {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md is out of date — run `skillet build`".into(),
        )];
    }

    for frag_name in &entry.fragments_used {
        let frag_path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        let current = match hash_file(&frag_path) {
            Ok(h) => h,
            Err(e) => {
                return vec![diag(
                    Severity::Error,
                    &source.name,
                    "stale-build",
                    format!("cannot hash fragment '{frag_name}': {e}"),
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

    let compiled_hash = match hash_file(&output_path) {
        Ok(h) => h,
        Err(e) => {
            return vec![diag(
                Severity::Error,
                &source.name,
                "stale-build",
                format!("cannot read SKILL.md: {e}"),
            )]
        }
    };

    if compiled_hash != entry.compiled_hash {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md is out of date — run `skillet build`".into(),
        )];
    }

    vec![]
}
