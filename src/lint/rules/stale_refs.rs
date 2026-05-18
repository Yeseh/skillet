//! Rules: `stale-path-ref`, `stale-command-ref`, `stale-skill-ref`.

use crate::config::SkilletConfig;
use crate::refs::TYPED_REF_RE;
use crate::workspace::{self, SkillSource};
use std::path::Path;

use super::{diag, Diagnostic, Severity};

pub fn check(
    source: &SkillSource,
    raw: &str,
    config: &SkilletConfig,
    all_sources: &[SkillSource],
    skills_src_dir: &Path,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    for caps in TYPED_REF_RE.captures_iter(raw) {
        let prefix = &caps[1];
        let value = caps[2].trim();

        match prefix {
            "ref" => {
                if !source.skill_dir.join(value).exists() {
                    diags.push(diag(
                        Severity::Error,
                        &source.name,
                        "stale-path-ref",
                        format!("ref path not found: '{value}'"),
                    ));
                }
            }
            "cmd" => {
                let cmd = value.split_whitespace().next().unwrap_or(value);
                let allowed = config.lint.allowed_commands.iter().any(|c| c == cmd);
                if !allowed && !workspace::is_on_path(cmd) {
                    diags.push(diag(
                        Severity::Warning,
                        &source.name,
                        "stale-command-ref",
                        format!("command '{cmd}' not found on PATH"),
                    ));
                }
            }
            "skill" => {
                if !all_sources.iter().any(|s| s.name == value)
                    && !skills_src_dir.join(value).is_dir()
                {
                    diags.push(diag(
                        Severity::Error,
                        &source.name,
                        "stale-skill-ref",
                        format!("skill '{value}' not found in workspace"),
                    ));
                }
            }
            _ => {} // var:: / env:: validated by build
        }
    }

    diags
}
