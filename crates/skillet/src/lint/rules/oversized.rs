//! Rules: `oversized-skill`, `oversized-description`, `oversized-fragment`.

use crate::config::SkilletConfig;
use crate::lint::pipeline::SourceFile;
use crate::lint::LintContext;
use crate::lockfile::Lockfile;
use crate::tokens::count_tokens;

use super::{diag, Diagnostic, Severity};

/// Checks whether the compiled `SKILL.md` activation cost exceeds the limit.
///
/// Uses cached `activation_tokens` from the lockfile when available — no
/// re-tokenization.  Falls back to tokenizing compiled text from `LintContext`
/// when there is no lockfile entry.
pub fn check_skill(
    source: &SourceFile,
    config: &SkilletConfig,
    lockfile: &Lockfile,
    ctx: &LintContext,
) -> Vec<Diagnostic> {
    let tokens = if let Some(entry) = lockfile.skills.get(&source.name) {
        if entry.activation_tokens > 0 {
            entry.activation_tokens
        } else {
            ctx.activation_tokens
                .get(&source.name)
                .copied()
                .unwrap_or(0)
        }
    } else {
        ctx.activation_tokens
            .get(&source.name)
            .copied()
            .unwrap_or(0)
    };

    if tokens > config.lint.max_activation_tokens {
        vec![diag(
            Severity::Warning,
            &source.name,
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
/// Uses the pre-parsed frontmatter from Phase 1 — no re-parsing.
pub fn check_description(source: &SourceFile, config: &SkilletConfig) -> Vec<Diagnostic> {
    let fm = match source.frontmatter.as_ref() {
        Some(fm) => fm,
        None => return vec![],
    };
    let text = format!(
        "{} {}",
        fm.name.as_deref().unwrap_or(""),
        fm.description.as_deref().unwrap_or("")
    );
    let tokens = count_tokens(&text, &config.build.tokenizer);
    if tokens > config.lint.max_discovery_tokens {
        vec![diag(
            Severity::Warning,
            &source.name,
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
pub fn check_fragments(config: &SkilletConfig, ctx: &LintContext) -> Vec<Diagnostic> {
    ctx.fragment_tokens
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
