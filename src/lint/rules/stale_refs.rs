//! Rules: `stale-path-ref`, `stale-command-ref`, `stale-skill-ref`.

use crate::config::SkilletConfig;
use crate::refs::TYPED_REF_RE;
use crate::workspace::{self, SkillSource};
use std::path::Path;

use super::{diag_located, Diagnostic, Severity};

pub fn check(
    source: &SkillSource,
    raw: &str,
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    skills_src_dir: &Path,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let file_path = source.source_path.to_string_lossy().to_string();

    for caps in TYPED_REF_RE.captures_iter(raw) {
        let m = caps.get(0).expect("full match");
        let line_no = (raw[..m.start()].bytes().filter(|&b| b == b'\n').count() + 1) as u32;
        let prefix = &caps[1];
        let value = caps[2].trim();

        match prefix {
            "ref" => {
                if !source.skill_dir.join(value).exists() {
                    diags.push(diag_located(
                        Severity::Error,
                        &source.name,
                        "stale-path-ref",
                        format!("ref path not found: '{value}'"),
                        Some(file_path.clone()),
                        Some(line_no),
                    ));
                }
            }
            "cmd" => {
                let cmd = value.split_whitespace().next().unwrap_or(value);
                let allowed = config.lint.allowed_commands.iter().any(|c| c == cmd);
                if !allowed && !workspace::is_on_path(cmd) {
                    diags.push(diag_located(
                        Severity::Warning,
                        &source.name,
                        "stale-command-ref",
                        format!("command '{cmd}' not found on PATH"),
                        Some(file_path.clone()),
                        Some(line_no),
                    ));
                }
            }
            "skill" => {
                if !all_sources.iter().any(|s| s.name == value)
                    && !skills_src_dir.join(value).is_dir()
                {
                    diags.push(diag_located(
                        Severity::Error,
                        &source.name,
                        "stale-skill-ref",
                        format!("skill '{value}' not found in workspace"),
                        Some(file_path.clone()),
                        Some(line_no),
                    ));
                }
            }
            "var" => {
                if !config.vars.contains_key(value) {
                    diags.push(diag_located(
                        Severity::Error,
                        &source.name,
                        "stale-var-ref",
                        format!("var '{value}' not declared in [vars]"),
                        Some(file_path.clone()),
                        Some(line_no),
                    ));
                }
            }
            "env" => {
                if !config.env.contains_key(value) {
                    diags.push(diag_located(
                        Severity::Error,
                        &source.name,
                        "stale-env-ref",
                        format!("env '{value}' not declared in [env]"),
                        Some(file_path.clone()),
                        Some(line_no),
                    ));
                }
            }
            _ => {}
        }
    }

    diags
}
