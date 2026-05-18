//! Compilation pipeline: `.skill` sources → `SKILL.md` output files.

use crate::config::SkilletConfig;
use crate::lockfile::{LockMeta, Lockfile, SkillEntry};
use crate::new::load_config;
use crate::workspace::{self, SkillSource};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use regex::Regex;
use sha2::Digest;
use std::path::Path;

/// Compiles `.skill` sources to `SKILL.md` files and updates `skillet.lock`.
///
/// Compiles only the named skill when `skill_name` is `Some`, or all skills
/// in the workspace when it is `None`.
///
/// # Errors
///
/// Returns an error if any skill fails to compile (missing fragment, undefined
/// var/env ref, missing file ref, or frontmatter name mismatch).
pub fn run(workspace: &Path, skill_name: Option<&str>) -> Result<()> {
    let config = load_config(workspace)?;
    let skills_dir = workspace.join(&config.workspace.skills_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);

    let sources = workspace::discover_skills(&skills_dir)?;

    let targets: Vec<&SkillSource> = match skill_name {
        Some(name) => {
            let found = sources.iter().find(|s| s.name == name);
            match found {
                Some(s) => vec![s],
                None => bail!("skill '{}' not found in workspace", name),
            }
        }
        None => sources.iter().collect(),
    };

    if targets.is_empty() {
        eprintln!("no skills found in {}", skills_dir.display());
        return Ok(());
    }

    let mut lockfile = crate::lockfile::read(workspace)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now().to_rfc3339(),
        tokenizer: config.build.tokenizer.clone(),
    });

    for source in &targets {
        compile_skill(source, &config, workspace, &fragments_dir, &mut lockfile)?;
        println!("built {}", source.name);
    }

    crate::lockfile::write(workspace, &lockfile)?;
    Ok(())
}

fn compile_skill(
    source: &SkillSource,
    config: &SkilletConfig,
    workspace: &Path,
    fragments_dir: &Path,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let raw = std::fs::read_to_string(&source.source_path)
        .with_context(|| format!("failed to read {}", source.source_path.display()))?;

    let (frontmatter, name, body) = parse_source(&raw)
        .with_context(|| format!("failed to parse {}", source.source_path.display()))?;

    if name != source.name {
        bail!(
            "frontmatter name '{}' does not match skill directory '{}'",
            name,
            source.name
        );
    }

    let skills_dir = workspace.join(&config.workspace.skills_dir);

    let (processed_body, fragments_used) = process_fragments(&body, fragments_dir)?;
    let compiled_body = process_refs(&processed_body, &source.skill_dir, config, &skills_dir)?;

    let output = format!("---\n{}\n---\n{}", frontmatter, compiled_body);
    let output_path = source.skill_dir.join("SKILL.md");
    std::fs::write(&output_path, &output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    let source_hash = hash_file(&source.source_path)?;
    let compiled_hash = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(output.as_bytes()))
    );

    lockfile.skills.insert(
        source.name.clone(),
        SkillEntry {
            source_hash,
            compiled_hash,
            fragments_used,
        },
    );

    Ok(())
}

/// Splits a `.skill` source into `(frontmatter_str, name, body)`.
fn parse_source(source: &str) -> Result<(String, String, String)> {
    // Strip UTF-8 BOM if present
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);

    if !source.starts_with("---\n") {
        bail!("source must begin with --- frontmatter delimiter");
    }

    let rest = &source[4..]; // skip opening "---\n"

    let close_pos = rest
        .find("\n---\n")
        .ok_or_else(|| anyhow::anyhow!("frontmatter closing --- not found"))?;

    let frontmatter = rest[..close_pos].to_string();
    let body = rest[close_pos + 5..].to_string(); // skip "\n---\n"

    let name = extract_name(&frontmatter)?;
    Ok((frontmatter, name, body))
}

/// Extracts the `name` field from a raw YAML frontmatter string.
fn extract_name(frontmatter: &str) -> Result<String> {
    let re = Regex::new(r#"(?m)^name:\s*["']?([^"'#\n]+?)["']?\s*$"#).unwrap();
    re.captures(frontmatter)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("frontmatter missing 'name' field"))
}

/// Expands `{{> fragment-name }}` include directives in `body`.
///
/// Returns the expanded body and the list of fragment names used.
fn process_fragments(body: &str, fragments_dir: &Path) -> Result<(String, Vec<String>)> {
    let re = Regex::new(r"^\{\{>\s*([\w-]+)\s*\}\}\s*$").unwrap();
    let mut fragments_used: Vec<String> = Vec::new();

    // Use split('\n') so a trailing '\n' produces a final empty element,
    // allowing join('\n') to faithfully reconstruct the original.
    let lines: Vec<&str> = body.split('\n').collect();
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());

    for &line in &lines {
        if let Some(caps) = re.captures(line) {
            let frag_name = &caps[1];
            let content = workspace::load_fragment(fragments_dir, frag_name)?;
            if !fragments_used.contains(&frag_name.to_string()) {
                fragments_used.push(frag_name.to_string());
            }
            // Inline the fragment content, preserving its own line structure.
            // trim_end_matches('\n') avoids doubling blank lines at the join.
            out_lines.push(content.trim_end_matches('\n').to_string());
        } else {
            out_lines.push(line.to_string());
        }
    }

    Ok((out_lines.join("\n"), fragments_used))
}

/// Transforms all backtick ref directives in `body`.
///
/// Errors are collected and returned together; warnings are printed to stderr
/// and execution continues.
fn process_refs(
    body: &str,
    skill_dir: &Path,
    config: &SkilletConfig,
    skills_dir: &Path,
) -> Result<String> {
    let re = Regex::new(r"`(ref|cmd|skill|var|env)::([^`]+)`").unwrap();
    let mut result = String::with_capacity(body.len());
    let mut last_end = 0;
    let mut errors: Vec<String> = Vec::new();

    for caps in re.captures_iter(body) {
        let m = caps.get(0).unwrap();
        result.push_str(&body[last_end..m.start()]);
        last_end = m.end();

        let prefix = &caps[1];
        let value = caps[2].trim();

        match prefix {
            "ref" => {
                if !skill_dir.join(value).exists() {
                    errors.push(format!("ref path not found: '{}'", value));
                }
                result.push('`');
                result.push_str(value);
                result.push('`');
            }
            "cmd" => {
                let cmd = value.split_whitespace().next().unwrap_or(value);
                if !is_on_path(cmd) {
                    eprintln!("warning: command '{}' not found on PATH", cmd);
                }
                result.push('`');
                result.push_str(value);
                result.push('`');
            }
            "skill" => {
                if !skills_dir.join(value).is_dir() {
                    errors.push(format!("skill '{}' not found in workspace", value));
                }
                result.push('`');
                result.push_str(value);
                result.push('`');
            }
            "var" => match config.vars.get(value) {
                Some(v) => result.push_str(v),
                None => {
                    errors.push(format!("var '{}' not declared in [vars]", value));
                    result.push_str(&caps[0]);
                }
            },
            "env" => match config.env.get(value) {
                Some(e) => result.push_str(&e.default),
                None => {
                    errors.push(format!("env '{}' not declared in [env]", value));
                    result.push_str(&caps[0]);
                }
            },
            _ => result.push_str(&caps[0]),
        }
    }

    result.push_str(&body[last_end..]);

    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }

    Ok(result)
}

/// Returns `true` if `cmd` is found as a file in any directory on `PATH`.
fn is_on_path(cmd: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(cmd).is_file())
}

/// Returns `"sha256:<hex>"` of the file at `path`.
fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {} for hashing", path.display()))?;
    Ok(format!("sha256:{}", hex::encode(sha2::Sha256::digest(&bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;
    use std::fs;
    use tempfile::TempDir;

    fn init_workspace(dir: &Path) {
        let cfg = SkilletConfig::default();
        fs::write(dir.join("skillet.toml"), cfg.to_toml().unwrap()).unwrap();
        fs::create_dir_all(dir.join(&cfg.workspace.skills_dir)).unwrap();
        fs::create_dir_all(dir.join(&cfg.workspace.fragments_dir)).unwrap();
    }

    // ── parse_source ────────────────────────────────────────────────────────

    #[test]
    fn parse_source_splits_frontmatter_name_and_body() {
        // Arrange
        let src = "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n";

        // Act
        let (fm, name, body) = parse_source(src).unwrap();

        // Assert
        assert_eq!(name, "my-skill");
        assert!(fm.contains("description"));
        assert!(body.contains("# My Skill"));
    }

    #[test]
    fn parse_source_errors_when_no_frontmatter_delimiter() {
        // Arrange
        let src = "# No frontmatter\n";

        // Act & Assert
        assert!(parse_source(src).is_err());
    }

    #[test]
    fn parse_source_errors_when_frontmatter_not_closed() {
        // Arrange
        let src = "---\nname: foo\n";

        // Act & Assert
        assert!(parse_source(src).is_err());
    }

    // ── process_fragments ───────────────────────────────────────────────────

    #[test]
    fn process_fragments_inlines_fragment_content() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("note.fragment.skill"),
            "## Note\nsome content\n",
        )
        .unwrap();
        let body = "intro\n{{> note }}\noutro\n";

        // Act
        let (result, used) = process_fragments(body, tmp.path()).unwrap();

        // Assert
        assert!(result.contains("## Note"));
        assert!(result.contains("some content"));
        assert!(result.contains("intro") && result.contains("outro"));
        assert_eq!(used, vec!["note"]);
    }

    #[test]
    fn process_fragments_errors_on_missing_fragment() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let body = "{{> missing }}\n";

        // Act & Assert
        assert!(process_fragments(body, tmp.path()).is_err());
    }

    // ── process_refs ────────────────────────────────────────────────────────

    #[test]
    fn process_refs_strips_ref_prefix_keeps_backticks() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("foo.sh"), "").unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");
        let body = "`ref::foo.sh`";

        // Act
        let result = process_refs(body, tmp.path(), &config, &skills_dir).unwrap();

        // Assert
        assert_eq!(result, "`foo.sh`");
    }

    #[test]
    fn process_refs_substitutes_var_without_backticks() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default(); // vars.project_name = "my-project"
        let skills_dir = tmp.path().join("skills");

        // Act
        let result = process_refs(
            "deploy to `var::project_name`",
            tmp.path(),
            &config,
            &skills_dir,
        )
        .unwrap();

        // Assert
        assert_eq!(result, "deploy to my-project");
    }

    #[test]
    fn process_refs_substitutes_env_default_without_backticks() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default(); // env.CI.default = "false"
        let skills_dir = tmp.path().join("skills");

        // Act
        let result =
            process_refs("ci: `env::CI`", tmp.path(), &config, &skills_dir).unwrap();

        // Assert
        assert_eq!(result, "ci: false");
    }

    #[test]
    fn process_refs_strips_cmd_prefix_keeps_backticks() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");

        // Act — "ls" is always on PATH in CI
        let result =
            process_refs("`cmd::ls -la`", tmp.path(), &config, &skills_dir).unwrap();

        // Assert
        assert_eq!(result, "`ls -la`");
    }

    #[test]
    fn process_refs_errors_on_missing_ref_path() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");

        // Act & Assert
        assert!(process_refs("`ref::missing.sh`", tmp.path(), &config, &skills_dir).is_err());
    }

    #[test]
    fn process_refs_errors_on_undeclared_var() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");

        // Act & Assert
        assert!(process_refs("`var::unknown`", tmp.path(), &config, &skills_dir).is_err());
    }

    #[test]
    fn process_refs_errors_on_undeclared_env() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");

        // Act & Assert
        assert!(process_refs("`env::UNKNOWN`", tmp.path(), &config, &skills_dir).is_err());
    }

    #[test]
    fn process_refs_errors_on_missing_skill_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Act & Assert
        assert!(process_refs("`skill::nope`", tmp.path(), &config, &skills_dir).is_err());
    }

    // ── run ─────────────────────────────────────────────────────────────────

    #[test]
    fn run_writes_skill_md_with_frontmatter_and_body() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("my-skill.skill"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), Some("my-skill")).unwrap();

        // Assert
        let skill_md = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(skill_md.starts_with("---\n"));
        assert!(skill_md.contains("# My Skill"));
    }

    #[test]
    fn run_updates_skillet_lock_with_skill_entry() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("my-skill.skill"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), None).unwrap();

        // Assert
        let lf = crate::lockfile::read(tmp.path()).unwrap();
        assert!(lf.skills.contains_key("my-skill"));
        assert!(lf.meta.is_some());
    }

    #[test]
    fn run_errors_when_frontmatter_name_mismatches_dir() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("my-skill.skill"),
            "---\nname: wrong-name\ndescription: \"\"\n---\n\n# body\n",
        )
        .unwrap();

        // Act
        let result = run(tmp.path(), Some("my-skill"));

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("wrong-name"));
    }

    #[test]
    fn run_errors_when_named_skill_not_found() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());

        // Act
        let result = run(tmp.path(), Some("nonexistent"));

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn run_expands_fragments_in_output() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        fs::write(
            tmp.path().join("skills/_fragments/note.fragment.skill"),
            "## Note\nfragment content\n",
        )
        .unwrap();
        let skill_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("my-skill.skill"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\n{{> note }}\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), Some("my-skill")).unwrap();

        // Assert
        let output = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(output.contains("## Note"));
        assert!(output.contains("fragment content"));
        assert!(!output.contains("{{> note }}"));
    }
}
