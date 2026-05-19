//! Rule: `stale-markdown-link` — validates path targets of markdown links in skill sources.

use crate::config::SkilletConfig;
use crate::lint::pipeline::SourceFile;

use super::{diag, diag_located, Diagnostic, Severity};

/// Validates markdown link targets using the pre-extracted links from Phase 2.
pub fn check(source: &SourceFile, config: &SkilletConfig) -> Vec<Diagnostic> {
    let file_path = source.source_path.to_string_lossy().to_string();
    let mut diags = Vec::new();

    for link in &source.parsed_refs.links {
        if link.is_url {
            if config.build.verify_urls {
                diags.push(diag(
                    Severity::Info,
                    &source.name,
                    "unverified-url-link",
                    format!("URL link detected (not verified): '{}'", link.target),
                ));
            }
        } else {
            let resolved = source.skill_dir.join(&link.target);
            if !resolved.exists() {
                diags.push(diag_located(
                    Severity::Error,
                    &source.name,
                    "stale-markdown-link",
                    format!(
                        "markdown link target not found: '{}' (resolved to '{}')",
                        link.target,
                        resolved.display()
                    ),
                    Some(file_path.clone()),
                    Some(link.line),
                    Some(link.col),
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
    use crate::lint::pipeline;
    use crate::workspace::SkillSource;
    use std::fs;
    use tempfile::TempDir;

    fn make_source_file(dir: &std::path::Path, name: &str, content: &str) -> pipeline::SourceFile {
        let skill_dir = dir.join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source_path = skill_dir.join(format!("{name}.pan"));
        fs::write(&source_path, content).unwrap();
        let skill_out_dir = dir.join("skills").join(name);
        let src = SkillSource {
            name: name.to_string(),
            source_path,
            skill_dir,
            skill_out_dir,
        };
        let files = pipeline::scan_sources(&[src], "cl100k_base");
        let (mut files, _) = pipeline::extract_refs(files, &[name]);
        files.remove(0)
    }

    #[test]
    fn check_passes_for_existing_path_link() {
        let tmp = TempDir::new().unwrap();
        let src = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [guide](guide.md)\n",
        );
        fs::write(src.skill_dir.join("guide.md"), "").unwrap();
        // Re-scan so parsed_refs are populated.
        let diags = check(&src, &SkilletConfig::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn check_errors_on_missing_path_link() {
        let tmp = TempDir::new().unwrap();
        let src = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [missing](missing.md)\n",
        );
        let diags = check(&src, &SkilletConfig::default());
        assert!(diags.iter().any(|d| d.rule == "stale-markdown-link"));
    }

    #[test]
    fn check_ignores_url_link_when_verify_urls_false() {
        let tmp = TempDir::new().unwrap();
        let src = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [docs](https://example.com)\n",
        );
        let diags = check(&src, &SkilletConfig::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn check_emits_info_for_url_link_when_verify_urls_true() {
        let tmp = TempDir::new().unwrap();
        let src = make_source_file(
            tmp.path(),
            "my-skill",
            "---\nname: my-skill\ndescription: x\n---\n\nSee [docs](https://example.com)\n",
        );
        let mut config = SkilletConfig::default();
        config.build.verify_urls = true;
        let diags = check(&src, &config);
        assert!(diags
            .iter()
            .any(|d| d.rule == "unverified-url-link" && d.severity == Severity::Info));
    }
}
