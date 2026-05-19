//! Rule: `stale-build` — verifies the compiled `SKILL.md` matches the source.

use crate::config::SkilletConfig;
use crate::workspace::SkillSource;
use std::path::Path;

use super::{diag, Diagnostic, Severity};

pub fn check(
    source: &SkillSource,
    config: &SkilletConfig,
    fragments_dir: &Path,
    skills_src_dir: &Path,
) -> Vec<Diagnostic> {
    let output_path = source.skill_out_dir.join("SKILL.md");

    if !output_path.exists() {
        return vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md not found — run `skillet build`".into(),
        )];
    }

    let expected =
        match crate::build::compile_to_string(source, config, fragments_dir, skills_src_dir) {
            Ok((s, _)) => s,
            Err(e) => {
                return vec![diag(
                    Severity::Error,
                    &source.name,
                    "stale-build",
                    format!("cannot verify build output: {e}"),
                )]
            }
        };

    match std::fs::read_to_string(&output_path) {
        Ok(on_disk) if on_disk == expected => vec![],
        Ok(_) => vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            "SKILL.md is out of date — run `skillet build`".into(),
        )],
        Err(e) => vec![diag(
            Severity::Error,
            &source.name,
            "stale-build",
            format!("cannot read SKILL.md: {e}"),
        )],
    }
}
