//! Compilation pipeline: `.pan` sources → `SKILL.md` output files.
//!
//! This module is pure — it accepts pre-loaded content and returns compiled
//! text without reading or writing files.

use crate::config::{EnvVar, SkilletConfig};
use crate::lockfile::{FragmentLockEntry, LockMeta, Lockfile, SkillEntry, SkillRefs};
use crate::refs::{extract_markdown_links, extract_path_refs, typed_refs, RefKind};
use crate::workspace::{self, SkillSource};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use gray_matter::{engine::YAML, Matter};
use owo_colors::OwoColorize;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

static FRAGMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\{\{>\s*([\w-]+)\s*\}\}\s*$").unwrap());

// ── Public types ───────────────────────────────────────────────────────────────

/// A named skill source unit — the content of a `.pan` file.
#[derive(Debug, Clone)]
pub struct SourceUnit {
    /// Skill name (must match the `name` field in frontmatter).
    pub name: String,
    /// Display path for diagnostics (e.g. `"src/skills/foo/foo.pan"`).
    pub path: String,
    /// Raw source file content.
    pub content: String,
}

/// Context required to compile a single skill.
#[non_exhaustive]
pub struct CompileContext {
    /// The skill source to compile.
    pub source: SourceUnit,
    /// Loaded fragment content keyed by fragment name (without `.fragment.pan`).
    pub fragments: HashMap<String, String>,
    /// Relative file paths that exist in the skill directory (for `ref::` validation).
    /// Pass an empty set to skip file-existence checks.
    pub known_files: HashSet<String>,
    /// Commands known to be available on `PATH` (for `cmd::` validation).
    /// Pass an empty set to skip command-existence checks.
    pub known_commands: HashSet<String>,
    /// Skill names present in the workspace (for `skill::` validation).
    /// Pass an empty set to skip skill-existence checks.
    pub known_skills: HashSet<String>,
    /// Template variable substitutions declared in `[vars]`.
    pub vars: BTreeMap<String, String>,
    /// Environment variable declarations from `[env]`.
    pub env: BTreeMap<String, EnvVar>,
    /// Tokenizer model (e.g. `"cl100k_base"`).
    pub tokenizer: String,
}

/// Result of compiling a single skill.
#[non_exhaustive]
#[derive(Debug)]
pub struct CompileResult {
    /// Compiled `SKILL.md` content.
    pub output: String,
    /// Fragment names inlined during compilation.
    pub fragments_used: Vec<String>,
    /// Structured ref inventory collected from compiled output.
    pub refs: SkillRefs,
    /// Token count for name + description (discovery cost).
    pub discovery_tokens: u32,
    /// Token count for the full compiled output (activation cost).
    pub activation_tokens: u32,
    /// Relative paths from `ref::` directives (for CLI to compute transitive tokens).
    pub ref_paths: Vec<String>,
    /// Warnings for `cmd::` refs whose commands were not in `known_commands`.
    pub cmd_warnings: Vec<BuildDiagnostic>,
}

// ── Diagnostics ────────────────────────────────────────────────────────────────

/// A single build-time diagnostic.
#[derive(Debug, Clone)]
pub struct BuildDiagnostic {
    /// Severity of the diagnostic.
    pub severity: BuildSeverity,
    /// Skill name this applies to.
    pub skill: String,
    /// Description of the problem.
    pub message: String,
    /// File path where the issue was found (display string).
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
        path: &str,
        line: u32,
        col: u32,
    ) -> Self {
        Self {
            severity,
            skill: skill.to_string(),
            message,
            path: path.to_string(),
            line,
            col,
        }
    }

    /// Renders this diagnostic as a plain-text one-liner (no ANSI colours).
    pub fn render_text(&self) -> String {
        let tag = match self.severity {
            BuildSeverity::Warning => "warning",
            BuildSeverity::Error => "error",
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
    /// The diagnostics that caused the build to fail.
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl BuildFailure {
    fn new(diagnostics: Vec<BuildDiagnostic>) -> Self {
        Self { diagnostics }
    }

    /// Renders all diagnostics as plain-text, one per line.
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

// ── Public API ─────────────────────────────────────────────────────────────────

/// Compiles a single `.pan` source into `SKILL.md` content.
///
/// All inputs are pre-loaded by the caller; this function performs no I/O.
///
/// # Errors
///
/// Returns a [`BuildFailure`] error when any ref directive cannot be resolved
/// (missing fragment, undefined var/env ref, missing file ref, or frontmatter
/// name mismatch).
pub fn compile(ctx: &CompileContext) -> Result<CompileResult> {
    let (frontmatter, name, body, body_start_line) = parse_source(&ctx.source.content)
        .with_context(|| format!("failed to parse {}", ctx.source.path))?;

    if name != ctx.source.name {
        bail!(
            "frontmatter name '{}' does not match skill directory '{}'",
            name,
            ctx.source.name
        );
    }

    let (processed_body, fragments_used) = process_fragments(&body, &ctx.fragments)?;
    let (compiled_body, cmd_warnings) = process_refs(
        &processed_body,
        &ctx.source.name,
        &ctx.source.path,
        body_start_line,
        ctx,
    )?;

    let output = format!("---\n{}\n---\n\n{}", frontmatter, compiled_body);
    let refs = collect_structured_refs(&output);

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
    let discovery_tokens = crate::tokens::count_tokens(&discovery_text, &ctx.tokenizer);
    let activation_tokens = crate::tokens::count_tokens(&output, &ctx.tokenizer);
    let ref_paths = extract_path_refs(&ctx.source.content);

    Ok(CompileResult {
        output,
        fragments_used,
        refs,
        discovery_tokens,
        activation_tokens,
        ref_paths,
        cmd_warnings,
    })
}

/// Renders the minimal `.pan` scaffold for a skill named `name`.
pub fn scaffold_content(name: &str) -> String {
    format!("---\nname: {name}\ndescription: \"TODO: describe this skill\"\n---\n\n# {name}\n")
}

// ── Internal helpers ───────────────────────────────────────────────────────────

/// Typed representation of a `.pan` file's YAML frontmatter.
#[derive(Deserialize)]
struct SkillFrontmatter {
    name: String,
}

/// Parses a `.pan` source with `gray_matter`, returning
/// `(frontmatter_str, name, body, body_start_line)`.
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
fn process_fragments(
    body: &str,
    fragments: &HashMap<String, String>,
) -> Result<(String, Vec<String>)> {
    let mut fragments_used: Vec<String> = Vec::new();

    let lines: Vec<&str> = body.split('\n').collect();
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());

    for &line in &lines {
        if let Some(caps) = FRAGMENT_RE.captures(line) {
            let frag_name = &caps[1];
            let content = fragments
                .get(frag_name)
                .ok_or_else(|| anyhow::anyhow!("fragment '{}' not found", frag_name))?;
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
            out_lines.push(content.trim_end_matches('\n').to_string());
        } else {
            out_lines.push(line.to_string());
        }
    }

    Ok((out_lines.join("\n"), fragments_used))
}

/// Transforms all backtick ref directives in `body`.
///
/// Returns `(processed_body, cmd_warnings)`.
fn process_refs(
    body: &str,
    skill_name: &str,
    source_path: &str,
    body_start_line: u32,
    ctx: &CompileContext,
) -> Result<(String, Vec<BuildDiagnostic>)> {
    let mut result = String::with_capacity(body.len());
    let mut last_end = 0;
    let mut errors: Vec<BuildDiagnostic> = Vec::new();
    let mut cmd_warnings: Vec<BuildDiagnostic> = Vec::new();

    for tr in typed_refs(body) {
        result.push_str(&body[last_end..tr.start]);
        last_end = tr.end;

        match tr.kind {
            RefKind::Ref => {
                if !ctx.known_files.is_empty() && !ctx.known_files.contains(&tr.value) {
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
                if !ctx.known_commands.is_empty() && !ctx.known_commands.contains(cmd) {
                    cmd_warnings.push(BuildDiagnostic::new(
                        BuildSeverity::Warning,
                        skill_name,
                        format!("command '{}' not found on PATH", cmd),
                        source_path,
                        body_start_line + tr.line - 1,
                        tr.col,
                    ));
                }
                result.push('`');
                result.push_str(&tr.value);
                result.push('`');
            }
            RefKind::Skill => {
                if !ctx.known_skills.is_empty() && !ctx.known_skills.contains(&tr.value) {
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
            RefKind::Var => match ctx.vars.get(&tr.value) {
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
            RefKind::Env => match ctx.env.get(&tr.value) {
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

    Ok((result, cmd_warnings))
}

/// Returns `"sha256:<hex>"` of `bytes` (in-memory hashing for compiled output).
pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
}

/// Collects all detectable refs from compiled SKILL.md text into a structured form.
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

// ── Build orchestration ────────────────────────────────────────────────────────

/// Output format for build results.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    /// Plain
    Text,
    /// Json-encoded BuildReport struct (for machine consumption).
    Json,
}

/// Options controlling the build step.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct BuildOptions {
    /// If true, skip network checks (e.g. URL verification).
    pub offline: bool,
    /// If true, treat URL verification warnings as errors.
    pub strict: bool,
    /// Output format for build results (text or JSON)
    pub format: OutputFormat,
}

impl BuildOptions {
    /// Creates BuildOptions with default output format (text).
    pub fn new(offline: bool, strict: bool) -> Self {
        Self {
            offline,
            strict,
            format: OutputFormat::Text,
        }
    }

    /// Creates BuildOptions with the specified output format.
    pub fn new_with_format(offline: bool, strict: bool, format: OutputFormat) -> Self {
        Self {
            offline,
            strict,
            format,
        }
    }
}

/// Structured report produced by a build run.
#[derive(Debug, Serialize)]
pub struct BuildReport {
    /// Names of skills that were built during this run.
    pub skills_built: Vec<String>,
    /// Warnings generated during the build (e.g. missing commands or broken URLs).
    pub warnings: Vec<String>,
    /// Path to the lockfile used during the build.
    pub lockfile_path: String,
}

/// Compiles `.pan` sources to `SKILL.md` files and updates `skillet.lock`.
pub fn run(
    workspace: &Path,
    skill_name: Option<&str>,
    opts: &BuildOptions,
    cfg: &SkilletConfig,
) -> Result<()> {
    let skills_src_dir = workspace.join(&cfg.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&cfg.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&cfg.workspace.fragments_dir);

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

    let fragments = load_all_fragments(&fragments_dir)?;
    let known_skills: HashSet<String> = sources.iter().map(|s| s.name.clone()).collect();

    let mut lockfile = crate::lockfile::read(workspace)?;
    lockfile.meta = Some(LockMeta {
        skillet_version: env!("CARGO_PKG_VERSION").to_string(),
        built_at: Utc::now(),
        tokenizer: cfg.build.tokenizer.clone(),
    });

    let mut skills_built: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for source in &targets {
        compile_one_skill(source, cfg, &fragments, &known_skills, &mut lockfile)?;
        if opts.format != OutputFormat::Json {
            println!("built {}", source.name);
        }
        skills_built.push(source.name.clone());
    }

    rebuild_fragment_entries(&mut lockfile, &fragments_dir, &cfg.build.tokenizer)?;

    let lock_path = workspace.join("skillet.lock");
    crate::lockfile::write(workspace, &lockfile)?;

    if cfg.build.verify_urls && !opts.offline {
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

fn load_all_fragments(fragments_dir: &Path) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    if !fragments_dir.exists() {
        return Ok(map);
    }
    for entry in WalkDir::new(fragments_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".fragment.pan"))
        {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read fragment '{}'", path.display()))?;
            map.insert(name.to_string(), content);
        }
    }
    Ok(map)
}

fn compile_one_skill(
    source: &SkillSource,
    cfg: &SkilletConfig,
    fragments: &HashMap<String, String>,
    known_skills: &HashSet<String>,
    lockfile: &mut Lockfile,
) -> Result<()> {
    let source_content = std::fs::read_to_string(&source.source_path)
        .with_context(|| format!("failed to read {}", source.source_path.display()))?;

    let known_files: HashSet<String> = WalkDir::new(&source.skill_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            e.path()
                .strip_prefix(&source.skill_dir)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .collect();

    let ctx = CompileContext {
        source: SourceUnit {
            name: source.name.clone(),
            path: source.source_path.to_string_lossy().to_string(),
            content: source_content.clone(),
        },
        fragments: fragments.clone(),
        known_files,
        known_commands: HashSet::new(),
        known_skills: known_skills.clone(),
        vars: cfg.vars.clone(),
        env: cfg.env.clone(),
        tokenizer: cfg.build.tokenizer.clone(),
    };

    let result = compile(&ctx)?;

    for w in &result.cmd_warnings {
        eprintln!("{}", w.render_text());
    }

    std::fs::create_dir_all(&source.skill_out_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            source.skill_out_dir.display()
        )
    })?;
    let output_path = source.skill_out_dir.join("SKILL.md");
    std::fs::write(&output_path, &result.output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    copy_skill_subfolders(&source.skill_dir, &source.skill_out_dir)?;

    let source_hash = workspace::hash_file(&source.source_path)?;
    let compiled_hash = hash_bytes(result.output.as_bytes());

    let old_minhash = lockfile
        .skills
        .get(&source.name)
        .filter(|e| e.compiled_hash == compiled_hash)
        .map(|e| e.minhash.clone())
        .unwrap_or_default();

    let ref_tokens: u32 = result
        .ref_paths
        .iter()
        .filter_map(|rel| {
            let path = source.skill_dir.join(rel);
            std::fs::read_to_string(&path)
                .ok()
                .map(|t| crate::tokens::count_tokens(&t, &cfg.build.tokenizer))
        })
        .sum();
    let transitive_tokens = result.activation_tokens + ref_tokens;

    lockfile.skills.insert(
        source.name.clone(),
        SkillEntry {
            source_hash,
            compiled_hash,
            discovery_tokens: result.discovery_tokens,
            activation_tokens: result.activation_tokens,
            transitive_tokens,
            fragments_used: result.fragments_used,
            refs: result.refs,
            minhash: old_minhash,
        },
    );

    Ok(())
}

fn rebuild_fragment_entries(
    lockfile: &mut Lockfile,
    fragments_dir: &Path,
    tokenizer: &str,
) -> Result<()> {
    lockfile.fragments.clear();

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

fn copy_skill_subfolders(skill_dir: &Path, skill_out_dir: &Path) -> Result<()> {
    for entry in WalkDir::new(skill_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sub_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let dest_sub_dir = skill_out_dir.join(&sub_name);
        if sub_name == "reference" {
            build_reference_dir(path, &dest_sub_dir)?;
        } else {
            workspace::copy_dir_recursive(path, &dest_sub_dir)?;
        }
    }
    Ok(())
}

fn build_reference_dir(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap();
        if rel == std::path::Path::new("") {
            continue;
        }
        if path.is_dir() {
            std::fs::create_dir_all(dest.join(rel))
                .with_context(|| format!("failed to create {}", dest.join(rel).display()))?;
        } else {
            let dest_file = if path.extension().and_then(|e| e.to_str()) == Some("pan") {
                dest.join(rel.with_extension("md"))
            } else {
                dest.join(rel)
            };
            if let Some(parent) = dest_file.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::copy(path, &dest_file).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    path.display(),
                    dest_file.display()
                )
            })?;
        }
    }
    Ok(())
}

fn verify_urls_from_lockfile(
    lockfile: &Lockfile,
    strict: bool,
    warnings: &mut Vec<String>,
    verbose: bool,
) -> Result<()> {
    use crate::net::url_verify::{verify_urls, UrlCheckResult};

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

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;

    fn default_ctx(name: &str, content: &str) -> CompileContext {
        let cfg = SkilletConfig::default();
        CompileContext {
            source: SourceUnit {
                name: name.to_string(),
                path: format!("{name}.pan"),
                content: content.to_string(),
            },
            fragments: HashMap::new(),
            known_files: HashSet::new(),
            known_commands: HashSet::new(),
            known_skills: HashSet::new(),
            vars: cfg.vars,
            env: cfg.env,
            tokenizer: cfg.build.tokenizer,
        }
    }

    // ── parse_source ───────────────────────────────────────────────────────────

    #[test]
    fn parse_source_splits_frontmatter_name_and_body() {
        let src = "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n";
        let (fm, name, body, _) = parse_source(src).unwrap();
        assert_eq!(name, "my-skill");
        assert!(fm.contains("description"));
        assert!(body.contains("# My Skill"));
    }

    #[test]
    fn parse_source_errors_when_frontmatter_missing() {
        let src = "# No frontmatter\n";
        assert!(parse_source(src).is_err());
    }

    #[test]
    fn parse_source_errors_when_name_field_absent() {
        let src = "---\ndescription: no name here\n---\n\n# body\n";
        assert!(parse_source(src).is_err());
    }

    // ── process_fragments ──────────────────────────────────────────────────────

    #[test]
    fn process_fragments_inlines_fragment_content() {
        let mut frags = HashMap::new();
        frags.insert("note".to_string(), "## Note\nsome content\n".to_string());
        let body = "intro\n{{> note }}\noutro\n";
        let (result, used) = process_fragments(body, &frags).unwrap();
        assert!(result.contains("## Note"));
        assert!(result.contains("some content"));
        assert!(result.contains("intro") && result.contains("outro"));
        assert_eq!(used, vec!["note"]);
    }

    #[test]
    fn process_fragments_errors_on_missing_fragment() {
        let body = "{{> missing }}\n";
        assert!(process_fragments(body, &HashMap::new()).is_err());
    }

    #[test]
    fn process_fragments_errors_on_nested_include() {
        let mut frags = HashMap::new();
        frags.insert("outer".to_string(), "## Outer\n{{> inner }}\n".to_string());
        let body = "{{> outer }}\n";
        let result = process_fragments(body, &frags);
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
        let mut frags = HashMap::new();
        frags.insert("safe".to_string(), "use `cmd::ls`\n".to_string());
        let body = "{{> safe }}\n";
        assert!(process_fragments(body, &frags).is_ok());
    }

    // ── process_refs ───────────────────────────────────────────────────────────

    #[test]
    fn process_refs_strips_ref_prefix_keeps_backticks() {
        let mut ctx = default_ctx("test-skill", "");
        ctx.known_files.insert("foo.sh".to_string());
        let (result, _) = process_refs("`ref::foo.sh`", "test-skill", "test.pan", 1, &ctx).unwrap();
        assert_eq!(result, "`foo.sh`");
    }

    #[test]
    fn process_refs_substitutes_var_without_backticks() {
        let ctx = default_ctx("test-skill", "");
        let (result, _) = process_refs(
            "deploy to `var::project_name`",
            "test-skill",
            "test.pan",
            1,
            &ctx,
        )
        .unwrap();
        assert_eq!(result, "deploy to my-project");
    }

    #[test]
    fn process_refs_substitutes_env_without_backticks() {
        let ctx = default_ctx("test-skill", "");
        let (result, _) = process_refs("ci: `env::CI`", "test-skill", "test.pan", 1, &ctx).unwrap();
        let expected = std::env::var("CI").unwrap_or_else(|_| "false".to_string());
        assert_eq!(result, format!("ci: {}", expected));
    }

    #[test]
    fn process_refs_strips_cmd_prefix_keeps_backticks() {
        let ctx = default_ctx("test-skill", "");
        let (result, _) = process_refs("`cmd::ls -la`", "test-skill", "test.pan", 1, &ctx).unwrap();
        assert_eq!(result, "`ls -la`");
    }

    #[test]
    fn process_refs_errors_on_missing_ref_path() {
        let mut ctx = default_ctx("test-skill", "");
        ctx.known_files.insert("dummy".to_string()); // non-empty set activates checking
        assert!(process_refs("`ref::missing.sh`", "test-skill", "test.pan", 1, &ctx).is_err());
    }

    #[test]
    fn process_refs_errors_on_undeclared_var() {
        let ctx = default_ctx("test-skill", "");
        assert!(process_refs("`var::unknown`", "test-skill", "test.pan", 1, &ctx).is_err());
    }

    #[test]
    fn process_refs_errors_on_undeclared_env() {
        let ctx = default_ctx("test-skill", "");
        assert!(process_refs("`env::UNKNOWN`", "test-skill", "test.pan", 1, &ctx).is_err());
    }

    #[test]
    fn process_refs_errors_on_missing_skill_ref() {
        let mut ctx = default_ctx("test-skill", "");
        ctx.known_skills.insert("dummy".to_string()); // non-empty set activates checking
        assert!(process_refs("`skill::nope`", "test-skill", "test.pan", 1, &ctx).is_err());
    }

    // ── compile ────────────────────────────────────────────────────────────────

    #[test]
    fn compile_produces_output_with_frontmatter_and_body() {
        let ctx = default_ctx(
            "my-skill",
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n",
        );
        let result = compile(&ctx).unwrap();
        assert!(result.output.starts_with("---\n"));
        assert!(result.output.contains("---\n\n"));
        assert!(result.output.contains("# My Skill"));
    }

    #[test]
    fn compile_errors_when_frontmatter_name_mismatches_dir() {
        let ctx = default_ctx(
            "my-skill",
            "---\nname: wrong-name\ndescription: \"\"\n---\n\n# body\n",
        );
        let result = compile(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("wrong-name"));
    }

    #[test]
    fn compile_expands_fragments_in_output() {
        let cfg = SkilletConfig::default();
        let mut fragments = HashMap::new();
        fragments.insert(
            "note".to_string(),
            "## Note\nfragment content\n".to_string(),
        );
        let ctx = CompileContext {
            source: SourceUnit {
                name: "my-skill".to_string(),
                path: "my-skill.pan".to_string(),
                content: "---\nname: my-skill\ndescription: \"\"\n---\n\n{{> note }}\n".to_string(),
            },
            fragments,
            known_files: HashSet::new(),
            known_commands: HashSet::new(),
            known_skills: HashSet::new(),
            vars: cfg.vars,
            env: cfg.env,
            tokenizer: cfg.build.tokenizer,
        };
        let result = compile(&ctx).unwrap();
        assert!(result.output.contains("## Note"));
        assert!(result.output.contains("fragment content"));
        assert!(!result.output.contains("{{> note }}"));
        assert_eq!(result.fragments_used, vec!["note"]);
    }

    #[test]
    fn compile_records_url_refs() {
        let ctx = default_ctx(
            "my-skill",
            "---\nname: my-skill\ndescription: \"\"\n---\n\nProject: `var::project_name`. See [docs](https://example.com)\n",
        );
        let result = compile(&ctx).unwrap();
        assert!(!result.refs.urls.is_empty());
    }

    // ── collect_structured_refs ────────────────────────────────────────────────

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
}
