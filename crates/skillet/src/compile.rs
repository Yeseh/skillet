//! Compatibility compile API for `.pan` → `SKILL.md`.
//!
//! This module preserves the legacy `skillet::compile` surface used by the
//! CLI/tests while delegating body compilation to the newer AST compiler in
//! `crate::compiler::compile`.
#![allow(missing_docs)]

use crate::compiler::compile::{
    compile_body, render_fragments, BodyCompileInput, CompileDiag, DiagSeverity,
};
use crate::config::EnvVar;
use crate::lockfile::SkillRefs;
use crate::refs::{extract_markdown_links, extract_path_refs, typed_refs, RefKind};
use anyhow::{bail, Context, Result};
use gray_matter::{engine::YAML, Matter};
use serde::Deserialize;
use sha2::Digest;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::LazyLock;

static LEGACY_FRAGMENT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\{\{>\s*([\w-]+)\s*\}\}\s*$").expect("valid fragment regex")
});

#[derive(Debug, Clone)]
pub struct SourceUnit {
    pub name: String,
    pub path: String,
    pub content: String,
}

pub struct CompileContext {
    pub source: SourceUnit,
    pub fragments: HashMap<String, String>,
    pub known_files: HashSet<String>,
    pub known_commands: HashSet<String>,
    pub known_skills: HashSet<String>,
    pub vars: BTreeMap<String, String>,
    pub env: BTreeMap<String, EnvVar>,
    pub tokenizer: String,
}

#[derive(Debug)]
pub struct CompileResult {
    pub output: String,
    pub fragments_used: Vec<String>,
    pub refs: SkillRefs,
    pub discovery_tokens: u32,
    pub activation_tokens: u32,
    pub ref_paths: Vec<String>,
    pub cmd_warnings: Vec<BuildDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct BuildDiagnostic {
    pub severity: BuildSeverity,
    pub skill: String,
    pub message: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSeverity {
    Warning,
    Error,
}

impl BuildDiagnostic {
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

#[derive(Debug)]
pub struct BuildFailure {
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl BuildFailure {
    fn new(diagnostics: Vec<BuildDiagnostic>) -> Self {
        Self { diagnostics }
    }

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

    let body = normalize_legacy_fragments(&body);
    let fragments: HashMap<String, String> = ctx
        .fragments
        .iter()
        .map(|(k, v)| (k.clone(), normalize_legacy_fragments(v)))
        .collect();
    let rendered_fragments = render_fragments(&fragments);

    let compiled = compile_body(&BodyCompileInput {
        body: &body,
        skill_name: &ctx.source.name,
        fragments: &rendered_fragments,
        vars: &ctx.vars,
        env: &ctx.env,
        known_skills: &ctx.known_skills,
        known_files: &ctx.known_files,
        known_commands: &ctx.known_commands,
        tokenizer: &ctx.tokenizer,
    });

    let mut cmd_warnings: Vec<BuildDiagnostic> = Vec::new();
    let mut errors: Vec<BuildDiagnostic> = Vec::new();
    for diag in &compiled.diagnostics {
        let mapped = map_diag(diag, &ctx.source.name, &ctx.source.path, body_start_line);
        match mapped.severity {
            BuildSeverity::Warning => cmd_warnings.push(mapped),
            BuildSeverity::Error => errors.push(mapped),
        }
    }

    if !errors.is_empty() {
        return Err(BuildFailure::new(errors).into());
    }

    let output = format!("---\n{}\n---\n\n{}", frontmatter, compiled.text);
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
        fragments_used: compiled.fragments_used,
        refs,
        discovery_tokens,
        activation_tokens,
        ref_paths,
        cmd_warnings,
    })
}

pub fn scaffold_content(name: &str) -> String {
    format!("---\nname: {name}\ndescription: \"TODO: describe this skill\"\n---\n\n# {name}\n")
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Default)]
pub struct BuildOptions {
    pub offline: bool,
    pub strict: bool,
    pub format: OutputFormat,
}

impl BuildOptions {
    pub fn new(offline: bool, strict: bool) -> Self {
        Self {
            offline,
            strict,
            format: OutputFormat::Text,
        }
    }

    pub fn new_with_format(offline: bool, strict: bool, format: OutputFormat) -> Self {
        Self {
            offline,
            strict,
            format,
        }
    }
}

#[derive(Deserialize)]
struct SkillFrontmatter {
    name: String,
}

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

fn normalize_legacy_fragments(text: &str) -> String {
    text.lines()
        .map(|line| {
            LEGACY_FRAGMENT_RE
                .captures(line)
                .map(|caps| format!("{{> {} <}}", &caps[1]))
                .unwrap_or_else(|| line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn map_diag(diag: &CompileDiag, skill: &str, path: &str, body_start_line: u32) -> BuildDiagnostic {
    let severity = match diag.severity {
        DiagSeverity::Warning => BuildSeverity::Warning,
        DiagSeverity::Error => BuildSeverity::Error,
    };
    BuildDiagnostic {
        severity,
        skill: skill.to_string(),
        message: diag.message.clone(),
        path: path.to_string(),
        line: body_start_line + diag.line - 1,
        col: diag.col,
    }
}

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

    #[test]
    fn compile_builds_output() {
        let ctx = default_ctx(
            "my-skill",
            "---\nname: my-skill\ndescription: \"\"\n---\n\n# My Skill\n",
        );

        let result = compile(&ctx).expect("compile should succeed");
        assert!(result.output.contains("# My Skill"));
    }

    #[test]
    fn compile_rejects_frontmatter_name_mismatch() {
        let ctx = default_ctx(
            "my-skill",
            "---\nname: wrong-name\ndescription: \"\"\n---\n\n# body\n",
        );

        let err = compile(&ctx).expect_err("compile should fail");
        assert!(err.to_string().contains("wrong-name"));
    }

    #[test]
    fn legacy_fragment_syntax_is_supported() {
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

        let result = compile(&ctx).expect("compile should succeed");
        assert!(result.output.contains("fragment content"));
        assert_eq!(result.fragments_used, vec!["note"]);
    }
}
