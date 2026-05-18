//! Rule: `stale-markdown-link` — validates path targets of markdown links in skill sources.
//!
//! Layer 1 detection: `[text](./path)` links are checked against the filesystem.
//! URL links (`[text](https://...)`) are only validated when `verify_urls = true`.

use crate::config::SkilletConfig;
use crate::refs::{extract_body, extract_markdown_links};
use crate::workspace::SkillSource;

use super::{diag, Diagnostic, Severity};

/// Checks all markdown links in `raw` for validity.
///
/// - File-path targets are resolved relative to the skill directory and
///   reported as errors when missing.
/// - URL targets produce a warning only when `config.build.verify_urls` is
///   `true`; URL reachability itself is not tested here (that is deferred to a
///   future story), so the warning merely records that the URL was detected.
pub fn check(source: &SkillSource, raw: &str, config: &SkilletConfig) -> Vec<Diagnostic> {
    let body = extract_body(raw);
    let mut diags = Vec::new();

    for link in extract_markdown_links(&body) {
        if link.is_url {
            // URL reachability verification is out of scope for this story;
            // emit an info note so authors know it was detected.
            if config.build.verify_urls {
                diags.push(diag(
                    Severity::Info,
                    &source.name,
                    "unverified-url-link",
                    format!("URL link detected (not verified): '{}'", link.target),
                ));
            }
        } else {
            // Resolve relative to skill directory
            let resolved = source.skill_dir.join(&link.target);
            if !resolved.exists() {
                diags.push(diag(
                    Severity::Error,
                    &source.name,
                    "stale-markdown-link",
                    format!(
                        "markdown link target not found: '{}' (resolved to '{}')",
                        link.target,
                        resolved.display()
                    ),
                ));
            }
        }
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;
    use crate::workspace::SkillSource;
    use std::fs;
    use tempfile::TempDir;

    fn make_source(dir: &std::path::Path, name: &str, content: &str) -> SkillSource {
        let skill_dir = dir.join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        fs::write(&source_path, content).unwrap();
        let skill_out_dir = dir.join("skills").join(name);
        SkillSource { name: name.to_string(), source_path, skill_dir, skill_out_dir }
    }

    #[test]
    fn check_passes_for_existing_path_link() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [guide](guide.md)\n",
        );
        fs::write(src.skill_dir.join("guide.md"), "").unwrap();
        let config = SkilletConfig::default();

        // Act
        let diags = check(&src, &fs::read_to_string(&src.source_path).unwrap(), &config);

        // Assert
        assert!(diags.is_empty());
    }

    #[test]
    fn check_errors_on_missing_path_link() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [missing](missing.md)\n",
        );
        let config = SkilletConfig::default();

        // Act
        let diags = check(&src, &fs::read_to_string(&src.source_path).unwrap(), &config);

        // Assert
        assert!(diags.iter().any(|d| d.rule == "stale-markdown-link"));
    }

    #[test]
    fn check_ignores_url_link_when_verify_urls_false() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [docs](https://example.com)\n",
        );
        let config = SkilletConfig::default(); // verify_urls = false

        // Act
        let diags = check(&src, &fs::read_to_string(&src.source_path).unwrap(), &config);

        // Assert
        assert!(diags.is_empty());
    }

    #[test]
    fn check_emits_info_for_url_link_when_verify_urls_true() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let src = make_source(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [docs](https://example.com)\n",
        );
        let mut config = SkilletConfig::default();
        config.build.verify_urls = true;

        // Act
        let diags = check(&src, &fs::read_to_string(&src.source_path).unwrap(), &config);

        // Assert
        assert!(diags.iter().any(|d| d.rule == "unverified-url-link" && d.severity == Severity::Info));
    }
}
