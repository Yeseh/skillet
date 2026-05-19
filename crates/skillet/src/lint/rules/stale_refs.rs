//! Rules: `stale-path-ref`, `stale-command-ref`, `stale-skill-ref`.

use crate::config::SkilletConfig;
use crate::refs::{ParsedRefs, RefKind};
use crate::workspace::{self, SkillSource};
use std::path::Path;

use super::{diag_located, Diagnostic, Severity};

pub fn check(
    source: &SkillSource,
    parsed: &ParsedRefs,
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    skills_src_dir: &Path,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let file_path = source.source_path.to_string_lossy().to_string();

    for tr in &parsed.typed {
        match tr.kind {
            RefKind::Ref if !source.skill_dir.join(&tr.value).exists() => {
                diags.push(diag_located(
                    Severity::Error,
                    &source.name,
                    "stale-path-ref",
                    format!("ref path not found: '{}'", tr.value),
                    Some(file_path.clone()),
                    Some(tr.line),
                    Some(tr.col),
                ));
            }
            RefKind::Ref => {}
            RefKind::Cmd => {
                let cmd = tr.value.split_whitespace().next().unwrap_or(&tr.value);
                let allowed = config.lint.allowed_commands.iter().any(|c| c == cmd);
                if !allowed && !workspace::is_on_path(cmd) {
                    diags.push(diag_located(
                        Severity::Warning,
                        &source.name,
                        "stale-command-ref",
                        format!("command '{cmd}' not found on PATH"),
                        Some(file_path.clone()),
                        Some(tr.line),
                        Some(tr.col),
                    ));
                }
            }
            RefKind::Skill
                if !all_sources.iter().any(|s| s.name == tr.value)
                    && !skills_src_dir.join(&tr.value).is_dir() =>
            {
                diags.push(diag_located(
                    Severity::Error,
                    &source.name,
                    "stale-skill-ref",
                    format!("skill '{}' not found in workspace", tr.value),
                    Some(file_path.clone()),
                    Some(tr.line),
                    Some(tr.col),
                ));
            }
            RefKind::Skill => {}
            RefKind::Var if !config.vars.contains_key(&tr.value) => {
                diags.push(diag_located(
                    Severity::Error,
                    &source.name,
                    "stale-var-ref",
                    format!("var '{}' not declared in [vars]", tr.value),
                    Some(file_path.clone()),
                    Some(tr.line),
                    Some(tr.col),
                ));
            }
            RefKind::Var => {}
            RefKind::Env if !config.env.contains_key(&tr.value) => {
                diags.push(diag_located(
                    Severity::Error,
                    &source.name,
                    "stale-env-ref",
                    format!("env '{}' not declared in [env]", tr.value),
                    Some(file_path.clone()),
                    Some(tr.line),
                    Some(tr.col),
                ));
            }
            RefKind::Env => {}
        }
    }

    diags
}
