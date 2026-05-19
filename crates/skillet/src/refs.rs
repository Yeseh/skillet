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

/// The kind of a typed ref directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// `ref::` — file-path reference.
    Ref,
    /// `cmd::` — shell command reference.
    Cmd,
    /// `skill::` — cross-skill reference.
    Skill,
    /// `var::` — workspace variable reference.
    Var,
    /// `env::` — environment variable reference.
    Env,
}

impl RefKind {
    fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "ref" => Some(Self::Ref),
            "cmd" => Some(Self::Cmd),
            "skill" => Some(Self::Skill),
            "var" => Some(Self::Var),
            "env" => Some(Self::Env),
            _ => None,
        }
    }
}

/// A parsed typed ref directive with its position in the source text.
#[derive(Debug, Clone)]
pub struct TypedRef {
    /// The kind of this ref.
    pub kind: RefKind,
    /// The value after the `::` separator (trimmed).
    pub value: String,
    /// Byte offset of the opening backtick in the source text.
    pub start: usize,
    /// Byte offset just past the closing backtick in the source text.
    pub end: usize,
    /// 1-based line number in the source text.
    pub line: u32,
    /// 1-based column number (character position) in the source text.
    pub col: u32,
}

/// Returns all typed ref directives found in `text`, in source order.
pub fn typed_refs(text: &str) -> Vec<TypedRef> {
    TYPED_REF_RE
        .captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            let kind = RefKind::from_prefix(&caps[1])?;
            let before = &text[..m.start()];
            let line = (before.bytes().filter(|&b| b == b'\n').count() + 1) as u32;
            let col = (before.rfind('\n').map_or(m.start(), |n| m.start() - n - 1) + 1) as u32;
            Some(TypedRef {
                kind,
                value: caps[2].trim().to_string(),
                start: m.start(),
                end: m.end(),
                line,
                col,
            })
        })
        .collect()
}

/// Matches `ref::` path directives only (used for transitive cost calculation).
static PATH_REF_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`ref::([^`]+)`").unwrap());

/// Matches any backtick-enclosed content that is not a typed ref directive.
pub(crate) static UNTYPED_BACKTICK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`\n]+)`").unwrap());

/// Matches standard markdown links: `[text](target)`.  Captures the target.
pub(crate) static MARKDOWN_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap());

/// A parsed markdown link target with classification and source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    /// Display text of the link.
    pub text: String,
    /// Raw target string (path or URL).
    pub target: String,
    /// Whether the target looks like a URL.
    pub is_url: bool,
    /// 1-based line number in the source text.
    pub line: u32,
    /// 1-based column number in the source text.
    pub col: u32,
}

/// Extracts all markdown link targets from `text`.
///
/// Each entry carries the display text, raw target, a URL flag, and the
/// 1-based line/col of the opening `[` within `text`.  Image links (`![]()`)
/// are included — the leading `!` is part of the display text.
pub fn extract_markdown_links(text: &str) -> Vec<MarkdownLink> {
    MARKDOWN_LINK_RE
        .captures_iter(text)
        .map(|caps| {
            let m = caps.get(0).unwrap();
            let (line, col) = line_col(text, m.start());
            let target = caps[2].trim().to_string();
            let is_url = target.starts_with("http://") || target.starts_with("https://");
            MarkdownLink {
                text: caps[1].to_string(),
                target,
                is_url,
                line,
                col,
            }
        })
        .collect()
}

/// An untyped backtick whose content has been classified by the Layer 3 heuristic.
#[derive(Debug, Clone)]
pub struct UntypedRef {
    /// Raw content between the backticks (trimmed).
    pub content: String,
    /// Heuristic classification (`"path"`, `"url"`, `"skill"`, `"command"`).
    pub inferred_kind: &'static str,
    /// 1-based line number in the source text.
    pub line: u32,
    /// 1-based column number in the source text.
    pub col: u32,
}

/// All refs collected from a single source file in one pass.
///
/// Build this with [`ParsedRefs::extract`] and pass it to lint rules so that
/// each rule reads pre-collected, pre-positioned data rather than re-scanning
/// the raw text.
#[derive(Debug, Default)]
pub struct ParsedRefs {
    /// Typed ref directives (`ref::`, `cmd::`, `skill::`, `var::`, `env::`).
    pub typed: Vec<TypedRef>,
    /// Markdown links (`[text](target)`).
    pub links: Vec<MarkdownLink>,
    /// Untyped backticks that the Layer 3 heuristic could classify.
    pub untyped: Vec<UntypedRef>,
}

impl ParsedRefs {
    /// Scans `raw` once and populates all three ref collections.
    ///
    /// `skill_names` is used by the untyped-backtick classifier to recognise
    /// cross-skill references.
    pub fn extract(raw: &str, skill_names: &[&str]) -> Self {
        let typed = typed_refs(raw);

        // Byte ranges occupied by typed refs — used to skip them in the
        // untyped backtick scan so we don't double-report.
        let typed_spans: Vec<(usize, usize)> =
            typed.iter().map(|r| (r.start, r.end)).collect();

        let links = extract_markdown_links(raw);

        let untyped = UNTYPED_BACKTICK_RE
            .captures_iter(raw)
            .filter_map(|caps| {
                let m = caps.get(0).unwrap();
                if typed_spans.iter().any(|&(s, e)| m.start() >= s && m.end() <= e) {
                    return None;
                }
                let content = caps[1].trim();
                let inferred_kind = classify_untyped(content, skill_names)?;
                let (line, col) = line_col(raw, m.start());
                Some(UntypedRef {
                    content: content.to_string(),
                    inferred_kind,
                    line,
                    col,
                })
            })
            .collect();

        Self { typed, links, untyped }
    }
}

/// Returns the 1-based (line, col) for `offset` bytes into `text`.
fn line_col(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset];
    let line = (before.bytes().filter(|&b| b == b'\n').count() + 1) as u32;
    let col = (before.rfind('\n').map_or(offset, |n| offset - n - 1) + 1) as u32;
    (line, col)
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
pub fn classify_untyped(content: &str, skill_names: &[&str]) -> Option<&'static str> {
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
        let is_cmd_like = cmd
            .chars()
            .all(|c| c.is_lowercase() || c == '-' || c == '_');
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
