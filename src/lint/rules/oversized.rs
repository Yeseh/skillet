//! Rules: `oversized-skill`, `oversized-description`, `oversized-fragment`.

use crate::config::SkilletConfig;
use crate::parse::parse_frontmatter;
use crate::tokens::count_tokens;
use crate::workspace::SkillSource;
use std::path::Path;

use super::{diag, Diagnostic, Severity};

pub fn check_skill(source: &SkillSource, config: &SkilletConfig) -> Vec<Diagnostic> {
    let output_path = source.skill_out_dir.join("SKILL.md");
    let Ok(content) = std::fs::read_to_string(&output_path) else {
        return vec![];
    };
    let tokens = count_tokens(&content, &config.build.tokenizer);
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

pub fn check_description(
    source: &SkillSource,
    raw: &str,
    config: &SkilletConfig,
) -> Vec<Diagnostic> {
    let Ok(Some(fm)) = parse_frontmatter(raw) else {
        return vec![];
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
