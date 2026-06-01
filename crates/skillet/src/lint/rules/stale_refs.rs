//! Rules: `stale-path-ref`, `stale-command-ref`, `stale-skill-ref`.

use crate::config::SkilletConfig;
use crate::lint::pipeline::SourceFile;
use crate::lint::LintContext;

use super::{diag_located, Diagnostic, Severity};

/// Validates all typed refs in the pre-extracted `parsed_refs` from Phase 2.
pub fn check(source: &SourceFile, config: &SkilletConfig, ctx: &LintContext) -> Vec<Diagnostic> {
    use crate::refs::RefKind;

    let mut diags = Vec::new();
    let file_path = source.source_path.to_string_lossy().to_string();
    let empty_set = std::collections::HashSet::new();
    let skill_files = ctx.skill_files.get(&source.name).unwrap_or(&empty_set);

    for tr in &source.parsed_refs.typed {
        match tr.kind {
            RefKind::Ref if !skill_files.contains(&tr.value) => {
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
                let allowed = config.allowed_commands.iter().any(|c| c == cmd);
                if !allowed && !ctx.known_commands.contains(cmd) {
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
            RefKind::Skill if !ctx.known_skill_dirs.contains(&tr.value) => {
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
