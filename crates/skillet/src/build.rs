//! Compilation pipeline: `.pan` sources → `SKILL.md` output files.

use crate::config::{self, SkilletConfig};
use crate::lockfile::{LockMeta, Lockfile, SkillEntry, SkillRefs};
use crate::refs::{extract_markdown_links, typed_refs, RefKind};
use crate::workspace::{self, SkillSource};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use gray_matter::{engine::YAML, Matter};
use owo_colors::OwoColorize;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fmt;
use std::path::Path;
use std::sync::LazyLock;

static FRAGMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\{\{>\s*([\w-]+)\s*\}\}\s*$").unwrap());

/// Options controlling the build step.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct BuildOptions {
    /// Skip URL verification regardless of `verify_urls` in `skillet.toml`.
    pub offline: bool,
    /// Promote URL-check warnings to errors (build fails if any URL is broken
    /// or unreachable).
    pub strict: bool,
    /// Output format.
    pub format: OutputFormat,
}

impl BuildOptions {
    /// Creates a new `BuildOptions` with both flags specified.
    pub fn new(offline: bool, strict: bool) -> Self {
        Self {
            offline,
            strict,
            format: OutputFormat::Text,
        }
    }

    /// Creates a new `BuildOptions` with all fields specified.
    pub fn new_with_format(offline: bool, strict: bool, format: OutputFormat) -> Self {
        Self {
            offline,
            strict,
            format,
        }
    }
}

/// Output format for build results.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    /// Default text output.
    #[default]
    Text,
    /// Machine-parseable JSON.
    Json,
}

/// Structured report produced by a build run.
#[derive(Debug, Serialize)]
pub struct BuildReport {
    /// Names of skills that were compiled successfully.
    pub skills_built: Vec<String>,
    /// Warning messages encountered during the build (URL checks, missing commands).
    pub warnings: Vec<String>,
    /// Path to the written `skillet.lock` file.
    pub lockfile_path: String,
}

/// A single build-time diagnostic.
#[derive(Debug, Clone)]
pub struct BuildDiagnostic {
    /// Severity of the diagnostic.
    pub severity: BuildSeverity,
    /// Skill name this applies to.
    pub skill: String,
    /// Description of the problem.
    pub message: String,
    /// File path where the issue was found.
    pub path: String,
    /// 1-based line number where the issue was found.
    pub line: u32,
    /// 1-based column number where the issue was found.
    pub col: u32,
}

/// Build diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSeverity {
    /// Non-fatal warning.
    Warning,
    /// Fatal validation error.
    Error,
}

impl BuildDiagnostic {
    fn new(
        severity: BuildSeverity,
        skill: &str,
        message: String,
        path: &Path,
        line: u32,
        col: u32,
    ) -> Self {
        Self {
            severity,
            skill: skill.to_string(),
            message,
            path: path.display().to_string(),
            line,
            col,
        }
    }

    fn render_text(&self) -> String {
        let tag = match self.severity {
            BuildSeverity::Warning => "warning".yellow().bold().to_string(),
            BuildSeverity::Error => "error".red().bold().to_string(),
        };
        format!(
            "[{tag}] {} {} ({}:{}:{})",
            self.skill, self.message, self.path, self.line, self.col
        )
    }
}

/// Build failure containing one or more diagnostics.
#[derive(Debug)]
pub struct BuildFailure {
    diagnostics: Vec<BuildDiagnostic>,
}

impl BuildFailure {
    fn new(diagnostics: Vec<BuildDiagnostic>) -> Self {
        Self { diagnostics }
    }

    /// Renders the failure in the same text shape used by lint diagnostics.
    pub fn render_text(&self) -> String {
        self.diagnostics
            .iter()
            .map(BuildDiagnostic::render_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl fmt::Display for BuildFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_text())
    }
}

impl std::error::Error for BuildFailure {}

/// Compiles `.pan` sources to `SKILL.md` files and updates `skillet.lock`.
///
/// Compiles only the named skill when `skill_name` is `Some`, or all skills
/// in the workspace when it is `None`.
///
/// When `config.build.verify_urls` is `true` and `opts.offline` is `false`,
/// all HTTP/HTTPS URLs referenced by the compiled skills are verified for
/// reachability.  Results are printed as warnings; with `opts.strict` any
/// broken or unreachable URL causes the build to fail.
///
/// # Errors
///
/// Returns an error if any skill fails to compile (missing fragment, undefined
/// var/env ref, missing file ref, or frontmatter name mismatch).
pub fn run(workspace: &Path, skill_name: Option<&str>, opts: &BuildOptions) -> Result<()> {
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
        if opts.format == OutputFormat::Json {
            let report = BuildReport {
                skills_built: vec![],
                warnings: vec![],
                lockfile_path: workspace.join("skillet.lock").to_string_lossy().to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            eprintln!("no skills found in {}", skills_src_dir.display());
        }
        return Ok(());
    }

    let mut lockfile = crate::lockfile::read(workspace)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: config.build.tokenizer.clone(),
    });

    let mut skills_built: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for source in &targets {
        compile_skill(
            source,
            &config,
            &fragments_dir,
            &skills_src_dir,
            &mut lockfile,
        )?;
        if opts.format != OutputFormat::Json {
            println!("built {}", source.name);
        }
        skills_built.push(source.name.clone());
    }

    rebuild_fragment_entries(&mut lockfile, &fragments_dir, &config.build.tokenizer)?;

    let lock_path = workspace.join("skillet.lock");
    crate::lockfile::write(workspace, &lockfile)?;

    // URL verification (opt-in via config, suppressible with --offline).
    if config.build.verify_urls && !opts.offline {
        verify_urls_from_lockfile(
            &lockfile,
            opts.strict,
            &mut warnings,
            opts.format != OutputFormat::Json,
        )?;
    }

    if opts.format == OutputFormat::Json {
        let report = BuildReport {
            skills_built,
            warnings,
            lockfile_path: lock_path.to_string_lossy().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

/// Collects all `url::` refs from the lockfile, verifies them, and prints
/// results.  Returns `Ok(())` unless `strict` is set and any URL failed.
fn verify_urls_from_lockfile(
    lockfile: &crate::lockfile::Lockfile,
    strict: bool,
    warnings: &mut Vec<String>,
    verbose: bool,
) -> Result<()> {
    use crate::net::url_verify::{verify_urls, UrlCheckResult};
    use owo_colors::OwoColorize;

    let urls: Vec<String> = lockfile
        .skills
        .values()
        .flat_map(|e| e.refs.urls.iter().cloned())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if urls.is_empty() {
        return Ok(());
    }

    if verbose {
        println!("checking {} URL(s)…", urls.len());
    }
    let outcomes = verify_urls(&urls);

    let mut had_error = false;
    for outcome in &outcomes {
        match &outcome.result {
            UrlCheckResult::Ok => {}
            UrlCheckResult::Broken(code) => {
                let msg = format!("broken-url: {} ({})", outcome.url, code);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} ({})",
                        "warning[broken-url]:".yellow(),
                        outcome.url,
                        code
                    );
                }
                had_error = true;
            }
            UrlCheckResult::PossiblyDown(code) => {
                let msg = format!("url-possibly-down: {} ({})", outcome.url, code);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} ({})",
                        "info[url-possibly-down]:".cyan(),
                        outcome.url,
                        code
                    );
                }
            }
            UrlCheckResult::Unreachable(reason) => {
                let msg = format!("unreachable-url: {} — {}", outcome.url, reason);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} — {}",
                        "warning[unreachable-url]:".yellow(),
                        outcome.url,
                        reason
                    );
                }
                had_error = true;
            }
            UrlCheckResult::Rejected(reason) => {
                let msg = format!("rejected-url: {} — {}", outcome.url, reason);
                warnings.push(msg);
                if verbose {
                    eprintln!(
                        "{} {} — {}",
                        "warning[rejected-url]:".yellow(),
                        outcome.url,
                        reason
                    );
                }
                had_error = true;
            }
        }
    }

    if strict && had_error {
        bail!("URL verification failed (--strict mode)");
    }

    Ok(())
}

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

    let (frontmatter, name, body, body_start_line) = parse_source(&raw)
        .with_context(|| format!("failed to parse {}", source.source_path.display()))?;

    if name != source.name {
        bail!(
            "frontmatter name '{}' does not match skill directory '{}'",
            name,
            source.name
        );
    }

    let (processed_body, fragments_used) = process_fragments(&body, fragments_dir)?;
    let compiled_body = process_refs(
        &processed_body,
        &source.name,
        &source.source_path,
        body_start_line,
        &source.skill_dir,
        config,
        skills_src_dir,
    )?;
    Ok((
        format!("---\n{}\n---\n\n{}", frontmatter, compiled_body),
        fragments_used,
    ))
}

fn compile_skill(
    source: &SkillSource,
    config: &SkilletConfig,
    fragments_dir: &Path,
    skills_src_dir: &Path,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let (output, fragments_used) =
        compile_to_string(source, config, fragments_dir, skills_src_dir)?;
    std::fs::create_dir_all(&source.skill_out_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            source.skill_out_dir.display()
        )
    })?;
    let output_path = source.skill_out_dir.join("SKILL.md");
    std::fs::write(&output_path, &output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    let source_hash = workspace::hash_file(&source.source_path)?;
    let compiled_hash = hash_bytes(output.as_bytes());

    // Preserve cached MinHash signature when the compiled output is unchanged.
    let old_minhash = lockfile
        .skills
        .get(&source.name)
        .filter(|e| e.compiled_hash == compiled_hash)
        .map(|e| e.minhash.clone())
        .unwrap_or_default();
    let refs = collect_structured_refs(&output);

    // Token counts
    let tokenizer = &config.build.tokenizer;
    let discovery_text = {
        use crate::parse::parse_frontmatter;
        match parse_frontmatter(&output) {
            Ok(Some(fm)) => format!(
                "{} {}",
                fm.name.unwrap_or_default(),
                fm.description.unwrap_or_default()
            ),
            _ => String::new(),
        }
    };
    let discovery_tokens = crate::tokens::count_tokens(&discovery_text, tokenizer);
    let activation_tokens = crate::tokens::count_tokens(&output, tokenizer);
    let source_text = std::fs::read_to_string(&source.source_path)?;
    let ref_tokens: u32 = crate::refs::extract_path_refs(&source_text)
        .into_iter()
        .filter_map(|rel| {
            let path = source.skill_dir.join(&rel);
            std::fs::read_to_string(&path)
                .ok()
                .map(|t| crate::tokens::count_tokens(&t, tokenizer))
        })
        .sum();
    let transitive_tokens = activation_tokens + ref_tokens;

    lockfile.skills.insert(
        source.name.clone(),
        SkillEntry {
            source_hash,
            compiled_hash,
            discovery_tokens,
            activation_tokens,
            transitive_tokens,
            fragments_used,
            refs,
            minhash: old_minhash,
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
fn parse_source(source: &str) -> Result<(String, String, String, u32)> {
    let matter = Matter::<YAML>::new();
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let parsed = matter
        .parse::<SkillFrontmatter>(source)
        .context("failed to parse skill source")?;

    let fm = parsed
        .data
        .ok_or_else(|| anyhow::anyhow!("source has no YAML frontmatter"))?;

    let body_start_line = find_body_start_line(source, &parsed.content);

    Ok((parsed.matter, fm.name, parsed.content, body_start_line))
}

fn find_body_start_line(source: &str, body: &str) -> u32 {
    let body_offset = find_body_offset(source);
    let content_offset = if body.is_empty() {
        body_offset
    } else {
        source[body_offset..]
            .find(body)
            .map(|offset| body_offset + offset)
            .unwrap_or(body_offset)
    };

    (source[..content_offset]
        .bytes()
        .filter(|&byte| byte == b'\n')
        .count()
        + 1) as u32
}

fn find_body_offset(source: &str) -> usize {
    let mut offset = 0;
    let mut line_no = 0;

    for line in source.split_inclusive('\n') {
        line_no += 1;
        offset += line.len();
        if line_no > 1 && line.trim_end_matches(['\r', '\n']) == "---" {
            return offset;
        }
    }

    0
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
    skill_name: &str,
    source_path: &Path,
    body_start_line: u32,
    skill_dir: &Path,
    config: &SkilletConfig,
    skills_dir: &Path,
) -> Result<String> {
    let mut result = String::with_capacity(body.len());
    let mut last_end = 0;
    let mut errors: Vec<BuildDiagnostic> = Vec::new();

    for tr in typed_refs(body) {
        result.push_str(&body[last_end..tr.start]);
        last_end = tr.end;

        match tr.kind {
            RefKind::Ref => {
                if !skill_dir.join(&tr.value).exists() {
                    errors.push(BuildDiagnostic::new(
                        BuildSeverity::Error,
                        skill_name,
                        format!("ref path not found: '{}'", tr.value),
                        source_path,
                        body_start_line + tr.line - 1,
                        tr.col,
                    ));
                }
                result.push('`');
                result.push_str(&tr.value);
                result.push('`');
            }
            RefKind::Cmd => {
                let cmd = tr.value.split_whitespace().next().unwrap_or(&tr.value);
                if !workspace::is_on_path(cmd) {
                    eprintln!(
                        "{}",
                        BuildDiagnostic::new(
                            BuildSeverity::Warning,
                            skill_name,
                            format!("command '{}' not found on PATH", cmd),
                            source_path,
                            body_start_line + tr.line - 1,
                            tr.col,
                        )
                        .render_text()
                    );
                }
                result.push('`');
                result.push_str(&tr.value);
                result.push('`');
            }
            RefKind::Skill => {
                if !skills_dir.join(&tr.value).is_dir() {
                    errors.push(BuildDiagnostic::new(
                        BuildSeverity::Error,
                        skill_name,
                        format!("skill '{}' not found in workspace", tr.value),
                        source_path,
                        body_start_line + tr.line - 1,
                        tr.col,
                    ));
                }
                result.push('`');
                result.push_str(&tr.value);
                result.push('`');
            }
            RefKind::Var => match config.vars.get(&tr.value) {
                Some(v) => result.push_str(v),
                None => {
                    errors.push(BuildDiagnostic::new(
                        BuildSeverity::Error,
                        skill_name,
                        format!("var '{}' not declared in [vars]", tr.value),
                        source_path,
                        body_start_line + tr.line - 1,
                        tr.col,
                    ));
                }
            },
            RefKind::Env => match config.env.get(&tr.value) {
                Some(e) => {
                    let resolved = std::env::var(&tr.value).unwrap_or_else(|_| e.default.clone());
                    result.push_str(&resolved);
                }
                None => {
                    errors.push(BuildDiagnostic::new(
                        BuildSeverity::Error,
                        skill_name,
                        format!("env '{}' not declared in [env]", tr.value),
                        source_path,
                        body_start_line + tr.line - 1,
                        tr.col,
                    ));
                }
            },
        }
    }

    result.push_str(&body[last_end..]);

    if !errors.is_empty() {
        return Err(BuildFailure::new(errors).into());
    }

    Ok(result)
}

/// Rebuilds `lockfile.fragments` from the current `lockfile.skills` data.
///
/// Clears the existing entries, builds the `used_by` reverse-map from every
/// skill's `fragments_used` list, then hashes each fragment file on disk.
/// Sorting `used_by` alphabetically ensures deterministic lockfile output.
fn rebuild_fragment_entries(
    lockfile: &mut Lockfile,
    fragments_dir: &Path,
    tokenizer: &str,
) -> Result<()> {
    lockfile.fragments.clear();

    // Reverse-map: fragment name → [skill names]
    for (skill_name, entry) in &lockfile.skills {
        for frag_name in &entry.fragments_used {
            lockfile
                .fragments
                .entry(frag_name.clone())
                .or_default()
                .used_by
                .push(skill_name.clone());
        }
    }

    // Hash each fragment file, compute token count, and sort used_by.
    for (frag_name, frag_entry) in &mut lockfile.fragments {
        let path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        if let Ok(text) = std::fs::read_to_string(&path) {
            frag_entry.hash = hash_bytes(text.as_bytes());
            frag_entry.tokens = crate::tokens::count_tokens(&text, tokenizer);
        } else if let Ok(h) = workspace::hash_file(&path) {
            frag_entry.hash = h;
        }
        frag_entry.used_by.sort();
    }

    Ok(())
}

/// Collects all detectable refs from compiled SKILL.md text into a structured form.
///
/// Gathers Layer 2 typed refs (`kind::value`) and Layer 1 markdown link targets.
/// Duplicates are removed and lists are sorted for deterministic lockfile output.
fn collect_structured_refs(text: &str) -> SkillRefs {
    let mut paths: Vec<String> = Vec::new();
    let mut commands: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();
    let mut urls: Vec<String> = Vec::new();

    for tr in typed_refs(text) {
        match tr.kind {
            RefKind::Ref => paths.push(tr.value),
            RefKind::Cmd => commands.push(tr.value),
            RefKind::Skill => skills.push(tr.value),
            RefKind::Var | RefKind::Env => {}
        }
    }

    for link in extract_markdown_links(text) {
        if link.is_url {
            urls.push(link.target);
        } else {
            paths.push(link.target);
        }
    }

    paths.sort();
    paths.dedup();
    commands.sort();
    commands.dedup();
    skills.sort();
    skills.dedup();
    urls.sort();
    urls.dedup();

    SkillRefs {
        paths,
        commands,
        skills,
        urls,
    }
}

/// Returns `"sha256:<hex>"` of `bytes` (in-memory hashing for compiled output).
fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
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
        let (fm, name, body, _body_start_line) = parse_source(src).unwrap();

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
        let result = process_refs(
            body,
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir,
        )
        .unwrap();

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
            "test-skill",
            tmp.path(),
            1,
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
        let result = process_refs(
            "ci: `env::CI`",
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir,
        )
        .unwrap();

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
        let result = process_refs(
            "`cmd::ls -la`",
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir,
        )
        .unwrap();

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
        assert!(process_refs(
            "`ref::missing.sh`",
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir
        )
        .is_err());
    }

    #[test]
    fn process_refs_errors_on_undeclared_var() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");

        // Act & Assert
        assert!(process_refs(
            "`var::unknown`",
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir
        )
        .is_err());
    }

    #[test]
    fn process_refs_errors_on_undeclared_env() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");

        // Act & Assert
        assert!(process_refs(
            "`env::UNKNOWN`",
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir
        )
        .is_err());
    }

    #[test]
    fn process_refs_errors_on_missing_skill_ref() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        let config = SkilletConfig::default();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Act & Assert
        assert!(process_refs(
            "`skill::nope`",
            "test-skill",
            tmp.path(),
            1,
            tmp.path(),
            &config,
            &skills_dir
        )
        .is_err());
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
        run(tmp.path(), Some("my-skill"), &Default::default()).unwrap();

        // Assert
        let skill_md = fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
        assert!(skill_md.starts_with("---\n"));
        assert!(
            skill_md.contains("---\n\n"),
            "blank line must follow closing ---"
        );
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
        run(tmp.path(), None, &Default::default()).unwrap();

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
        let result = run(tmp.path(), Some("my-skill"), &Default::default());

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
        let result = run(tmp.path(), Some("nonexistent"), &Default::default());

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
        run(tmp.path(), Some("my-skill"), &Default::default()).unwrap();

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
        assert!(
            msg.contains("nested"),
            "error should mention 'nested': {msg}"
        );
        assert!(
            msg.contains("outer"),
            "error should name the fragment: {msg}"
        );
    }

    #[test]
    fn process_fragments_allows_fragment_content_without_includes() {
        // Arrange — fragment has backticks but no include directives
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("safe.fragment.pan"), "use `cmd::ls`\n").unwrap();
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
        run(tmp.path(), Some("alpha"), &Default::default()).unwrap();
        let lf = crate::lockfile::read(tmp.path()).unwrap();

        // Assert
        let frag = lf
            .fragments
            .get("note")
            .expect("'note' fragment entry missing");
        assert!(!frag.hash.is_empty(), "fragment hash should be set");
        assert!(
            frag.hash.starts_with("sha256:"),
            "hash should be sha256 prefixed"
        );
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
        run(tmp.path(), None, &Default::default()).unwrap();
        let lf = crate::lockfile::read(tmp.path()).unwrap();

        // Assert
        let frag = lf
            .fragments
            .get("shared")
            .expect("'shared' fragment entry missing");
        assert!(frag.used_by.contains(&"skill-a".to_string()));
        assert!(frag.used_by.contains(&"skill-b".to_string()));
    }

    // ── collect_structured_refs ──────────────────────────────────────────────

    #[test]
    fn collect_refs_includes_typed_ref_directive() {
        let text = "Use `cmd::git` for version control.";
        let refs = collect_structured_refs(text);
        assert!(refs.commands.contains(&"git".to_string()));
    }

    #[test]
    fn collect_refs_includes_markdown_path_link() {
        let text = "See [guide](./docs/guide.md).";
        let refs = collect_structured_refs(text);
        assert!(refs.paths.contains(&"./docs/guide.md".to_string()));
    }

    #[test]
    fn collect_refs_includes_markdown_url_link() {
        let text = "Visit [site](https://example.com).";
        let refs = collect_structured_refs(text);
        assert!(refs.urls.contains(&"https://example.com".to_string()));
    }

    #[test]
    fn collect_refs_deduplicates_entries() {
        let text = "`cmd::git` and `cmd::git`";
        let refs = collect_structured_refs(text);
        assert_eq!(
            refs.commands.iter().filter(|r| r.as_str() == "git").count(),
            1
        );
    }

    #[test]
    fn run_records_refs_in_lockfile() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let skill_src_dir = tmp.path().join("src/skills/my-skill");
        fs::create_dir_all(&skill_src_dir).unwrap();
        // skill references a declared var and a markdown URL
        fs::write(
            skill_src_dir.join("my-skill.pan"),
            "---\nname: my-skill\ndescription: \"\"\n---\n\nProject: `var::project_name`. See [docs](https://example.com)\n",
        )
        .unwrap();

        // Act
        run(tmp.path(), Some("my-skill"), &Default::default()).unwrap();
        let lf = crate::lockfile::read(tmp.path()).unwrap();

        // Assert
        let entry = lf.skills.get("my-skill").expect("skill entry missing");
        // var:: expands inline (no longer in compiled output as a directive),
        // but the markdown URL link should be recorded.
        assert!(
            !entry.refs.urls.is_empty(),
            "expected a url ref in lockfile, got: {:?}",
            entry.refs.urls
        );
    }
}
