//! Rules: `oversized-skill`, `oversized-description`, `oversized-fragment`.

use crate::config::SkilletConfig;
use crate::workspace::Workspace;

use super::{diag, CompiledSkill, Diagnostic, Severity};

/// Checks whether the compiled `SKILL.md` activation cost exceeds the limit.
///
/// Uses `activation_tokens` straight from the compile stage — no re-tokenizing.
pub fn check_skill(cs: &CompiledSkill, config: &SkilletConfig) -> Vec<Diagnostic> {
    let tokens = cs.output.activation_tokens;
    if tokens > config.lint.max_activation_tokens {
        vec![diag(
            Severity::Warning,
            &cs.name,
            "oversized-skill",
            format!(
                "activation ~{tokens} tokens exceeds limit of {}",
                config.lint.max_activation_tokens
            ),
        )]
    } else {
        vec![]
    }
}

/// Checks whether the skill description exceeds the discovery token limit.
///
/// Uses `discovery_tokens` straight from the compile stage — no re-tokenizing.
pub fn check_description(cs: &CompiledSkill, config: &SkilletConfig) -> Vec<Diagnostic> {
    let tokens = cs.output.discovery_tokens;
    if tokens > config.lint.max_discovery_tokens {
        vec![diag(
            Severity::Warning,
            &cs.name,
            "oversized-description",
            format!(
                "discovery ~{tokens} tokens exceeds limit of {}",
                config.lint.max_discovery_tokens
            ),
        )]
    } else {
        vec![]
    }
}

/// Checks whether any fragment file exceeds the fragment token limit.
pub fn check_fragments(ws: &Workspace, config: &SkilletConfig) -> Vec<Diagnostic> {
    ws.fragment_tokens
        .iter()
        .filter_map(|(frag_name, &tokens)| {
            if tokens > config.lint.max_fragment_tokens {
                Some(diag(
                    Severity::Warning,
                    "<workspace>",
                    "oversized-fragment",
                    format!(
                        "fragment '{frag_name}' is ~{tokens} tokens (limit: {})",
                        config.lint.max_fragment_tokens
                    ),
                ))
            } else {
                None
            }
        })
        .collect()
}
