//! Rules: `oversized-skill`, `oversized-description`, `oversized-fragment`.

use crate::config::SkilletConfig;
use crate::lint::pipeline::SourceFile;
use crate::lockfile::Lockfile;
use crate::tokens::count_tokens;
use std::path::Path;

use super::{diag, Diagnostic, Severity};

/// Checks whether the compiled `SKILL.md` activation cost exceeds the limit.
///
/// Uses cached `activation_tokens` from the lockfile when available — no
/// re-tokenization.  Falls back to reading `SKILL.md` directly when there is
/// no lockfile entry (e.g. a stale-build scenario where the rule still runs).
pub fn check_skill(source: &SourceFile, config: &SkilletConfig, lockfile: &Lockfile) -> Vec<Diagnostic> {
    let tokens = if let Some(entry) = lockfile.skills.get(&source.name) {
        if entry.activation_tokens > 0 {
            entry.activation_tokens
        } else {
            // Lockfile entry exists but tokens are zero (shouldn't happen for a
            // real build, but fall back gracefully).
            read_compiled_tokens(source, config)
        }
    } else {
        // No lockfile entry — stale-build will fire separately; still check
        // size by reading the compiled output.
        read_compiled_tokens(source, config)
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

fn read_compiled_tokens(source: &SourceFile, config: &SkilletConfig) -> u32 {
    let path = source.skill_out_dir.join("SKILL.md");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 0;
    };
    count_tokens(&content, &config.build.tokenizer)
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
pub fn check_fragments(config: &SkilletConfig, fragments_dir: &Path) -> Vec<Diagnostic> {
    if !fragments_dir.exists() {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(fragments_dir) else {
        return vec![];
    };

    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let fname = path.file_name()?.to_string_lossy().into_owned();
            let frag_name = fname.strip_suffix(".fragment.pan")?.to_string();
            let content = std::fs::read_to_string(&path).ok()?;
            let tokens = count_tokens(&content, &config.build.tokenizer);
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
