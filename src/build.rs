//! Compilation pipeline: `.pan` sources → `SKILL.md` output files.

use crate::config::{self, SkilletConfig};
use crate::lockfile::{FragmentLockEntry, LockMeta, Lockfile, SkillEntry};
use crate::workspace::{self, SkillSource};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use gray_matter::{engine::YAML, Matter};
use regex::Regex;
use serde::Deserialize;
use sha2::Digest;
use std::path::Path;
use std::sync::LazyLock;

static FRAGMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\{\{>\s*([\w-]+)\s*\}\}\s*$").unwrap());

static REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`(ref|cmd|skill|var|env)::([^`]+)`").unwrap());

/// Compiles `.pan` sources to `SKILL.md` files and updates `skillet.lock`.
///
/// Compiles only the named skill when `skill_name` is `Some`, or all skills
/// in the workspace when it is `None`.
///
/// # Errors
///
/// Returns an error if any skill fails to compile (missing fragment, undefined
/// var/env ref, missing file ref, or frontmatter name mismatch).
pub fn run(workspace: &Path, skill_name: Option<&str>) -> Result<()> {
    let config = config::load(workspace)?;
    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);

    let sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;

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
        eprintln!("no skills found in {}", skills_src_dir.display());
        return Ok(());
    }

    let mut lockfile = crate::lockfile::read(workspace)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: config.build.tokenizer.clone(),
    });

    for source in &targets {
        compile_skill(source, &config, &fragments_dir, &skills_src_dir, &mut lockfile)?;
        println!("built {}", source.name);
    }

    rebuild_fragment_entries(&mut lockfile, &fragments_dir)?;

    crate::lockfile::write(workspace, &lockfile)?;
    Ok(())
}

/// Compiles a skill source to a `String` without writing to disk or touching the lockfile.
///
/// Returns the compiled content and the list of fragment names inlined.
/// Used by `skillet lint` to verify that `SKILL.md` is up to date.
pub fn compile_to_string(
    source: &SkillSource,
    config: &SkilletConfig,
    fragments_dir: &Path,
    skills_src_dir: &Path,
) -> Result<(String, Vec<String>)> {
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

    let (processed_body, fragments_used) = process_fragments(&body, fragments_dir)?;
    let compiled_body = process_refs(&processed_body, &source.skill_dir, config, skills_src_dir)?;
    Ok((format!("---\n{}\n---\n{}", frontmatter, compiled_body), fragments_used))
}

fn compile_skill(
    source: &SkillSource,
    config: &SkilletConfig,
    fragments_dir: &Path,
    skills_src_dir: &Path,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let (output, fragments_used) = compile_to_string(source, config, fragments_dir, skills_src_dir)?;
    std::fs::create_dir_all(&source.skill_out_dir)
        .with_context(|| format!("failed to create output directory {}", source.skill_out_dir.display()))?;
    let output_path = source.skill_out_dir.join("SKILL.md");
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

/// Typed representation of a `.pan` file's YAML frontmatter.
#[derive(Deserialize)]
struct SkillFrontmatter {
    /// Skill identifier — must match the containing directory name.
    name: String,
}

/// Parses a `.pan` source with `gray_matter`, returning
/// `(frontmatter_str, name, body)`.
///
/// `frontmatter_str` is the raw YAML text between the `---` delimiters,
/// preserved verbatim for pass-through into `SKILL.md`.
fn parse_source(source: &str) -> Result<(String, String, String)> {
    let matter = Matter::<YAML>::new();
    let parsed = matter
        .parse::<SkillFrontmatter>(source.strip_prefix('\u{feff}').unwrap_or(source))
        .context("failed to parse skill source")?;

    let fm = parsed
        .data
        .ok_or_else(|| anyhow::anyhow!("source has no YAML frontmatter"))?;

    Ok((parsed.matter, fm.name, parsed.content))
}

/// Expands `{{> fragment-name }}` include directives in `body`.
///
/// Returns the expanded body and the list of fragment names used.
fn process_fragments(body: &str, fragments_dir: &Path) -> Result<(String, Vec<String>)> {
    let mut fragments_used: Vec<String> = Vec::new();

    // split('\n') so a trailing '\n' produces a final empty element,
    // allowing join('\n') to faithfully reconstruct the original.
    let lines: Vec<&str> = body.split('\n').collect();
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());

    for &line in &lines {
        if let Some(caps) = FRAGMENT_RE.captures(line) {
            let frag_name = &caps[1];
            let content = workspace::load_fragment(fragments_dir, frag_name)?;
            // Reject nested includes — keep fragment includes flat (v1 decision).
            for (lineno, frag_line) in content.lines().enumerate() {
                if FRAGMENT_RE.is_match(frag_line) {
                    bail!(
                        "fragment '{}' contains a nested fragment include on line {} — \
                         nesting is not supported (keep includes flat)",
                        frag_name,
                        lineno + 1
                    );
                }
            }
            if !fragments_used.iter().any(|f| f == frag_name) {
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
    let mut result = String::with_capacity(body.len());
    let mut last_end = 0;
    let mut errors: Vec<String> = Vec::new();

    for caps in REF_RE.captures_iter(body) {
        let m = caps.get(0).expect("captures_iter always yields a full match");
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
                if !workspace::is_on_path(cmd) {
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
                Some(e) => {
                    let resolved =
                        std::env::var(value).unwrap_or_else(|_| e.default.clone());
                    result.push_str(&resolved);
                }
                None => {
                    errors.push(format!("env '{}' not declared in [env]", value));
                    result.push_str(&caps[0]);
                }
            },
            _ => unreachable!("REF_RE only matches ref|cmd|skill|var|env"),
        }
    }

    result.push_str(&body[last_end..]);

    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }

    Ok(result)
}

/// Rebuilds `lockfile.fragments` from the current `lockfile.skills` data.
///
/// Clears the existing entries, builds the `used_by` reverse-map from every
/// skill's `fragments_used` list, then hashes each fragment file on disk.
/// Sorting `used_by` alphabetically ensures deterministic lockfile output.
fn rebuild_fragment_entries(lockfile: &mut Lockfile, fragments_dir: &Path) -> Result<()> {
    lockfile.fragments.clear();

    // Reverse-map: fragment name → [skill names]
    for (skill_name, entry) in &lockfile.skills {
        for frag_name in &entry.fragments_used {
            lockfile
                .fragments
                .entry(frag_name.clone())
                .or_insert_with(FragmentLockEntry::default)
                .used_by
                .push(skill_name.clone());
        }
    }

    // Hash each fragment file and sort used_by for deterministic output.
    for (frag_name, frag_entry) in &mut lockfile.fragments {
        let path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        if let Ok(h) = hash_file(&path) {
            frag_entry.hash = h;
        }
        frag_entry.used_by.sort();
    }

    Ok(())
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
        fs::create_dir_all(dir.join(&cfg.workspace.skills_src_dir)).unwrap();
        fs::create_dir_all(dir.join(&cfg.workspace.skills_out_dir)).unwrap();
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
    fn parse_source_errors_when_frontmatter_missing() {
        // Arrange — no --- delimiters at all
        let src = "# No frontmatter\n";

        // Act & Assert
        assert!(parse_source(src).is_err());
    }

    #[test]
    fn parse_source_errors_when_name_field_absent() {
        // Arrange
        let src = "---\ndescription: no name here\n---\n\n# body\n";

        // Act & Assert
        assert!(parse_source(src).is_err());
    }

    // ── process_fragments ───────────────────────────────────────────────────

    #[test]
    fn process_fragments_inlines_fragment_content() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("note.fragment.pan"),
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
    fn process_refs_substitutes_env_without_backticks() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default(); // env.CI.default = "false"
        let skills_dir = tmp.path().join("skills");

        // Act
        let result =
            process_refs("ci: `env::CI`", tmp.path(), &config, &skills_dir).unwrap();

        // Assert — resolves to live env var or falls back to the configured default
        let expected = std::env::var("CI").unwrap_or_else(|_| "false".to_string());
        assert_eq!(result, format!("ci: {}", expected));
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
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), Some("my-skill")).unwrap();

        // Assert
        let skill_md = fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
        assert!(skill_md.starts_with("---\n"));
        assert!(skill_md.contains("# My Skill"));
    }

    #[test]
    fn run_updates_skillet_lock_with_skill_entry() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
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
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
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
            tmp.path().join("src/skills/_fragments/note.fragment.pan"),
            "## Note\nfragment content\n",
        )
        .unwrap();
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\n{{> note }}\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), Some("my-skill")).unwrap();

        // Assert
        let output = fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
        assert!(output.contains("## Note"));
        assert!(output.contains("fragment content"));
        assert!(!output.contains("{{> note }}"));
    }

    // ── process_fragments: nested include detection ───────────────────────────

    #[test]
    fn process_fragments_errors_on_nested_include() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        // outer.fragment.pan contains a nested include directive
        fs::write(
            tmp.path().join("outer.fragment.pan"),
            "## Outer\n{{> inner }}\n",
        )
        .unwrap();
        let body = "{{> outer }}\n";

        // Act
        let result = process_fragments(body, tmp.path());

        // Assert
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("nested"), "error should mention 'nested': {msg}");
        assert!(msg.contains("outer"), "error should name the fragment: {msg}");
    }

    #[test]
    fn process_fragments_allows_fragment_content_without_includes() {
        // Arrange — fragment has backticks but no include directives
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("safe.fragment.pan"),
            "use `cmd::ls`\n",
        )
        .unwrap();
        let body = "{{> safe }}\n";

        // Act
        let result = process_fragments(body, tmp.path());

        // Assert
        assert!(result.is_ok());
    }

    // ── rebuild_fragment_entries ─────────────────────────────────────────────

    #[test]
    fn rebuild_fragment_entries_populates_hash_and_used_by() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        fs::write(
            tmp.path().join("src/skills/_fragments/note.fragment.pan"),
            "## Note\nfragment content\n",
        )
        .unwrap();
        let skill_src_dir = tmp.path().join("src/skills/alpha");
        fs::create_dir_all(&skill_src_dir).unwrap();
        fs::write(
            skill_src_dir.join("alpha.pan"),
            "---\nname: alpha\ndescription: \"\"\n---\n\n{{> note }}\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), Some("alpha")).unwrap();
        let lf = crate::lockfile::read(tmp.path()).unwrap();

        // Assert
        let frag = lf.fragments.get("note").expect("'note' fragment entry missing");
        assert!(!frag.hash.is_empty(), "fragment hash should be set");
        assert!(frag.hash.starts_with("sha256:"), "hash should be sha256 prefixed");
        assert_eq!(frag.used_by, vec!["alpha"], "used_by should list 'alpha'");
    }

    #[test]
    fn rebuild_fragment_entries_lists_all_skills_for_shared_fragment() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        fs::write(
            tmp.path().join("src/skills/_fragments/shared.fragment.pan"),
            "## Shared\ncontent\n",
        )
        .unwrap();
        for skill in &["skill-a", "skill-b"] {
            let dir = tmp.path().join("src/skills").join(skill);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(format!("{skill}.pan")),
                format!("---\nname: {skill}\ndescription: \"\"\n---\n\n{{{{> shared }}}}\n"),
            )
            .unwrap();
        }

        // Act
        run(tmp.path(), None).unwrap();
        let lf = crate::lockfile::read(tmp.path()).unwrap();

        // Assert
        let frag = lf.fragments.get("shared").expect("'shared' fragment entry missing");
        assert!(frag.used_by.contains(&"skill-a".to_string()));
        assert!(frag.used_by.contains(&"skill-b".to_string()));
    }
}
