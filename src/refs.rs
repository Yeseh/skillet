//! Ref parsing, classification, and heuristic inference.
//!
//! Centralises all three ref-related regex patterns and the Layer 3
//! heuristic so every caller — build, lint, and budget — works from a
//! single definition.

use gray_matter::{engine::YAML, Matter};
use regex::Regex;
use std::sync::LazyLock;

/// Matches all typed ref directives: `ref::`, `cmd::`, `skill::`, `var::`, `env::`.
pub(crate) static TYPED_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`(ref|cmd|skill|var|env)::([^`]+)`").unwrap());

/// Matches `ref::` path directives only (used for transitive cost calculation).
static PATH_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`ref::([^`]+)`").unwrap());

/// Matches any backtick-enclosed content that is not a typed ref directive.
pub(crate) static UNTYPED_BACKTICK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`\n]+)`").unwrap());

/// Matches standard markdown links: `[text](target)`.  Captures the target.
pub(crate) static MARKDOWN_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap());

/// A parsed markdown link target with classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    /// Display text of the link.
    pub text: String,
    /// Raw target string (path or URL).
    pub target: String,
    /// Whether the target looks like a URL.
    pub is_url: bool,
}

/// Extracts all markdown link targets from `text`.
///
/// Each entry carries the display text, raw target, and a flag indicating
/// whether the target is a URL (`http://` / `https://` prefix).  Image links
/// (`![]()`) are included — the leading `!` is part of the display text.
pub fn extract_markdown_links(text: &str) -> Vec<MarkdownLink> {
    MARKDOWN_LINK_RE
        .captures_iter(text)
        .map(|caps| {
            let target = caps[2].trim().to_string();
            let is_url =
                target.starts_with("http://") || target.starts_with("https://");
            MarkdownLink { text: caps[1].to_string(), target, is_url }
        })
        .collect()
}

/// Extracts the `ref::` path values from `text`.
///
/// Used by the budget module to accumulate transitive token costs.
pub fn extract_path_refs(text: &str) -> Vec<String> {
    PATH_REF_RE
        .captures_iter(text)
        .map(|caps| caps[1].trim().to_string())
        .collect()
}

/// Classifies untyped backtick content using the Layer 3 heuristics.
///
/// Returns a short type label (`"path"`, `"url"`, `"skill"`, `"command"`) when
/// the content looks like a recognisable ref type, or `None` when it cannot be
/// classified.  The label doubles as the suggested ref prefix in lint messages.
pub fn classify_untyped<'a>(content: &str, skill_names: &[&'a str]) -> Option<&'static str> {
    if content.starts_with("http://") || content.starts_with("https://") {
        return Some("url");
    }
    let path_exts = [
        ".sh", ".py", ".rs", ".toml", ".json", ".yaml", ".yml", ".md", ".txt", ".ts", ".js",
    ];
    if content.contains('/') || path_exts.iter().any(|e| content.ends_with(e)) {
        return Some("path");
    }
    if skill_names.contains(&content) {
        return Some("skill");
    }
    // Command heuristic: lowercase/hyphenated first word + flag-like second token.
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() >= 2 {
        let cmd = parts[0];
        let is_cmd_like = cmd.chars().all(|c| c.is_lowercase() || c == '-' || c == '_');
        let has_flag = parts[1..].iter().any(|p| p.starts_with('-'));
        if is_cmd_like && has_flag {
            return Some("command");
        }
    }
    None
}

/// Extracts the markdown body from a `.pan` source, stripping the YAML frontmatter.
pub fn extract_body(raw: &str) -> String {
    let matter = Matter::<YAML>::new();
    matter
        .parse::<gray_matter::Pod>(raw)
        .map(|p| p.content)
        .unwrap_or_else(|_| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_path_refs_finds_ref_values() {
        let text = "See `ref::./scripts/foo.sh` and `ref::./bar.md`.";
        let refs = extract_path_refs(text);
        assert_eq!(refs, vec!["./scripts/foo.sh", "./bar.md"]);
    }

    #[test]
    fn extract_path_refs_ignores_other_typed_refs() {
        let text = "`cmd::ls -la` and `skill::diagnose`";
        assert!(extract_path_refs(text).is_empty());
    }

    #[test]
    fn classify_untyped_detects_url() {
        assert_eq!(classify_untyped("https://example.com", &[]), Some("url"));
        assert_eq!(classify_untyped("http://x.com/path", &[]), Some("url"));
    }

    #[test]
    fn classify_untyped_detects_path_by_extension() {
        assert_eq!(classify_untyped("run.sh", &[]), Some("path"));
        assert_eq!(classify_untyped("config.toml", &[]), Some("path"));
    }

    #[test]
    fn classify_untyped_detects_path_by_separator() {
        assert_eq!(classify_untyped("./scripts/foo", &[]), Some("path"));
    }

    #[test]
    fn classify_untyped_detects_skill_name() {
        let names = ["diagnose", "caveman"];
        assert_eq!(classify_untyped("diagnose", &names), Some("skill"));
    }

    #[test]
    fn classify_untyped_detects_command_with_flag() {
        assert_eq!(classify_untyped("git bisect --run", &[]), Some("command"));
    }

    #[test]
    fn classify_untyped_returns_none_for_plain_word() {
        assert_eq!(classify_untyped("something", &[]), None);
    }

    #[test]
    fn extract_markdown_links_detects_path_link() {
        let text = "See [the guide](./docs/guide.md) for details.";
        let links = extract_markdown_links(text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "./docs/guide.md");
        assert!(!links[0].is_url);
    }

    #[test]
    fn extract_markdown_links_detects_url_link() {
        let text = "Visit [example](https://example.com).";
        let links = extract_markdown_links(text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "https://example.com");
        assert!(links[0].is_url);
    }

    #[test]
    fn extract_markdown_links_returns_empty_for_plain_text() {
        let links = extract_markdown_links("no links here");
        assert!(links.is_empty());
    }

    #[test]
    fn extract_markdown_links_detects_multiple_links() {
        let text = "[a](path/a.md) and [b](https://b.com)";
        let links = extract_markdown_links(text);
        assert_eq!(links.len(), 2);
        assert!(!links[0].is_url);
        assert!(links[1].is_url);
    }

    #[test]
    fn extract_body_strips_frontmatter() {
        let raw = "---\nname: foo\ndescription: bar\n---\n\n# Body\n";
        let body = extract_body(raw);
        assert!(body.contains("# Body"));
        assert!(!body.contains("name: foo"));
    }
}
