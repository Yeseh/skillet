//! Stage 4 — AST-based compiler for `.pan` body text.
//!
//! Design decisions locked in before this was written:
//! - **Plain `match`** over AST nodes — no Visitor trait.
//! - **Pre-pass** renders fragments once per workspace into a [`RenderedFragments`]
//!   struct before the per-file compilation loop begins.
//! - **Main pass** interpolates the pre-rendered strings and resolves refs
//!   inline while walking the node list once.
//! - **Single tiktoken pass** over the fully assembled output string at the end.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::EnvVar;

use super::{
    parse::{Node, PanParse, RefKind},
    PanSource,
};

// ── Public types ───────────────────────────────────────────────────────────────

/// Severity of a compile-time diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagSeverity {
    /// Build will fail.
    Error,
    /// Build succeeds but the issue should be addressed.
    Warning,
}

/// A diagnostic produced by [`compile_body`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileDiag {
    /// Severity level.
    pub severity: DiagSeverity,
    /// Human-readable description of the problem.
    pub message: String,
    /// 1-based line number in the body text.
    pub line: u32,
    /// 1-based column number in the body text.
    pub col: u32,
}

/// Fragment content pre-rendered for an entire workspace.
///
/// Build this **once** with [`render_fragments`] before starting the
/// per-file compilation loop.  Pass a reference to every [`compile_body`]
/// call so that fragment parsing is never repeated across files.
#[derive(Clone)]
pub struct RenderedFragments {
    /// Fragment id → trimmed content, ready to interpolate.
    pub rendered: HashMap<String, String>,
    /// Fragment id → reason string for fragments that cannot be expanded
    /// (e.g. they contain a nested include directive).
    pub poisoned: HashMap<String, String>,
}

/// All inputs needed to compile a single `.pan` body (post-frontmatter text).
pub struct BodyCompileInput<'a> {
    /// Body text — everything after the closing `---` of the YAML frontmatter.
    pub body: &'a str,
    /// Skill name used in diagnostics.
    pub skill_name: &'a str,
    /// Fragments pre-rendered once for the whole workspace via [`render_fragments`].
    pub fragments: &'a RenderedFragments,
    /// Workspace variable substitutions declared in `[vars]`.
    pub vars: &'a BTreeMap<String, String>,
    /// Declared environment variables with their default values.
    pub env: &'a BTreeMap<String, EnvVar>,
    /// Known skill names for `skill::` validation (empty set = skip validation).
    pub known_skills: &'a HashSet<String>,
    /// Known relative file paths for `ref::` validation (empty set = skip).
    pub known_files: &'a HashSet<String>,
    /// Known commands on `PATH` for `cmd::` validation (empty set = skip).
    pub known_commands: &'a HashSet<String>,
    /// Tiktoken encoding name (e.g. `"cl100k_base"`).
    pub tokenizer: &'a str,
}

/// Output of [`compile_body`].
pub struct BodyCompileOutput {
    /// Compiled markdown body text.
    pub text: String,
    /// Token count over `text` — produced by a single tiktoken pass.
    pub tokens: u32,
    /// Fragment ids spliced in during compilation, in first-use order.
    pub fragments_used: Vec<String>,
    /// All diagnostics collected.  Check for [`DiagSeverity::Error`] to
    /// determine whether the compilation should be treated as failed.
    pub diagnostics: Vec<CompileDiag>,
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Compiles a `.pan` body in three phases:
///
/// 1. **Pre-pass** — caller builds a [`RenderedFragments`] once for the whole
///    workspace via [`render_fragments`] and passes it in.  No fragment parsing
///    happens inside this function.
/// 2. **Main pass** — parses `input.body` into an AST, then walks every node
///    with a plain `match`, interpolating pre-rendered fragments and resolving
///    typed refs inline.
/// 3. **Token pass** — calls `count_tokens` exactly once on the fully
///    assembled output string.
///
/// Diagnostics are accumulated throughout; no phase aborts early.  The caller
/// should inspect `output.diagnostics` for [`DiagSeverity::Error`] entries.
pub fn compile_body(input: &BodyCompileInput<'_>) -> BodyCompileOutput {
    let mut diagnostics: Vec<CompileDiag> = Vec::new();

    // ── Phase 2: parse then walk ──────────────────────────────────────────────
    let source = PanSource::new(input.body.to_string(), None);
    let mut parser = PanParse::new(&source);
    parser.parse();

    // Carry forward any parse errors as compile diagnostics.
    for pe in &parser.errors {
        let (line, col) = offset_to_line_col(input.body, pe.range.start as usize);
        diagnostics.push(CompileDiag {
            severity: DiagSeverity::Error,
            message: format!("parse error: {:?}", pe.kind),
            line,
            col,
        });
    }

    let mut out = String::with_capacity(input.body.len());
    let mut fragments_used: Vec<String> = Vec::new();

    for node in &parser.nodes {
        match node {
            Node::Body { source_range } => {
                out.push_str(span(input.body, source_range));
            }

            // ``content`` → `content`  (double-backtick escape collapses to single)
            Node::EscapedBody { source_range } => {
                let raw = span(input.body, source_range);
                out.push('`');
                out.push_str(&raw[2..raw.len() - 2]);
                out.push('`');
            }

            // `something-unrecognised` — pass through verbatim.
            Node::RefSuspect { source_range } => {
                out.push_str(span(input.body, source_range));
            }

            Node::Fragment {
                value,
                source_range,
            } => {
                let id = value.trim();
                let (line, col) = offset_to_line_col(input.body, source_range.start as usize);
                if let Some(reason) = input.fragments.poisoned.get(id) {
                    diagnostics.push(CompileDiag {
                        severity: DiagSeverity::Error,
                        message: format!("cannot expand fragment '{}': {}", id, reason),
                        line,
                        col,
                    });
                } else if let Some(content) = input.fragments.rendered.get(id) {
                    out.push_str(content);
                    if !fragments_used.iter().any(|f| f == id) {
                        fragments_used.push(id.to_string());
                    }
                } else {
                    diagnostics.push(CompileDiag {
                        severity: DiagSeverity::Error,
                        message: format!("fragment '{}' not found", id),
                        line,
                        col,
                    });
                }
            }

            Node::Ref {
                kind,
                value,
                source_range,
            } => {
                let (line, col) = offset_to_line_col(input.body, source_range.start as usize);
                emit_ref(&mut out, kind, value, line, col, input, &mut diagnostics);
            }
        }
    }

    // ── Phase 3: single tiktoken pass ─────────────────────────────────────────
    let tokens = crate::tokens::count_tokens(&out, input.tokenizer);

    BodyCompileOutput {
        text: out,
        tokens,
        fragments_used,
        diagnostics,
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────────

/// Pre-renders all fragments for an entire workspace.
///
/// Call this **once** before compiling any files, then pass the returned
/// [`RenderedFragments`] to every [`compile_body`] call.  Each fragment's
/// content is parsed to detect nested `{> ... <}` directives; fragments that
/// contain nesting go into `poisoned` instead of `rendered` so that every
/// skill file that tries to use them gets a clear, located error rather than
/// a generic "not found".
///
/// Trailing newlines are stripped from rendered content.
pub fn render_fragments(raw: &HashMap<String, String>) -> RenderedFragments {
    let mut rendered: HashMap<String, String> = HashMap::with_capacity(raw.len());
    let mut poisoned: HashMap<String, String> = HashMap::new();

    for (id, content) in raw {
        let frag_src = PanSource::new(content.clone(), None);
        let mut parser = PanParse::new(&frag_src);
        parser.parse();

        let nested = parser
            .nodes
            .iter()
            .find(|n| matches!(n, Node::Fragment { .. }));

        match nested {
            Some(Node::Fragment {
                value: nested_id, ..
            }) => {
                poisoned.insert(
                    id.clone(),
                    format!(
                        "nested include detected (it includes '{}') — nesting is not supported",
                        nested_id.trim()
                    ),
                );
            }
            _ => {
                rendered.insert(id.clone(), content.trim_end_matches('\n').to_string());
            }
        }
    }

    RenderedFragments { rendered, poisoned }
}

/// Emits one typed ref into `out`, appending diagnostics for validation failures.
fn emit_ref(
    out: &mut String,
    kind: &RefKind,
    value: &str,
    line: u32,
    col: u32,
    input: &BodyCompileInput<'_>,
    diagnostics: &mut Vec<CompileDiag>,
) {
    match kind {
        RefKind::Reference => {
            if !input.known_files.is_empty() && !input.known_files.contains(value) {
                diagnostics.push(CompileDiag {
                    severity: DiagSeverity::Error,
                    message: format!("ref path not found: '{}'", value),
                    line,
                    col,
                });
            }
            out.push('`');
            out.push_str(value);
            out.push('`');
        }

        RefKind::Cmd => {
            let cmd = value.split_whitespace().next().unwrap_or(value);
            if !input.known_commands.is_empty() && !input.known_commands.contains(cmd) {
                diagnostics.push(CompileDiag {
                    severity: DiagSeverity::Warning,
                    message: format!("command '{}' not found on PATH", cmd),
                    line,
                    col,
                });
            }
            out.push('`');
            out.push_str(value);
            out.push('`');
        }

        RefKind::Skill => {
            if !input.known_skills.is_empty() && !input.known_skills.contains(value) {
                diagnostics.push(CompileDiag {
                    severity: DiagSeverity::Error,
                    message: format!("skill '{}' not found in workspace", value),
                    line,
                    col,
                });
            }
            out.push('`');
            out.push_str(value);
            out.push('`');
        }

        // agent::, path::, url:: — pass the value through as a backtick span.
        RefKind::Agent | RefKind::Path | RefKind::Url => {
            out.push('`');
            out.push_str(value);
            out.push('`');
        }

        RefKind::Var => match input.vars.get(value) {
            Some(v) => out.push_str(v),
            None => diagnostics.push(CompileDiag {
                severity: DiagSeverity::Error,
                message: format!("var '{}' not declared in [vars]", value),
                line,
                col,
            }),
        },

        RefKind::Env => match input.env.get(value) {
            Some(e) => {
                let resolved = std::env::var(value).unwrap_or_else(|_| e.default.clone());
                out.push_str(&resolved);
            }
            None => diagnostics.push(CompileDiag {
                severity: DiagSeverity::Error,
                message: format!("env '{}' not declared in [env]", value),
                line,
                col,
            }),
        },
    }
}

/// Borrows a substring of `src` at the given byte range.
#[inline]
fn span<'a>(src: &'a str, range: &std::ops::Range<u32>) -> &'a str {
    &src[range.start as usize..range.end as usize]
}

/// Converts a byte `offset` in `src` to a 1-based `(line, col)` pair.
fn offset_to_line_col(src: &str, offset: usize) -> (u32, u32) {
    let capped = offset.min(src.len());
    let before = &src[..capped];
    let line = (before.bytes().filter(|&b| b == b'\n').count() + 1) as u32;
    let col = (before.rfind('\n').map_or(capped, |n| capped - n - 1) + 1) as u32;
    (line, col)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SkilletConfig;

    fn default_input<'a>(
        body: &'a str,
        frags: &'a RenderedFragments,
        cfg: &'a SkilletConfig,
        known_skills: &'a HashSet<String>,
        known_files: &'a HashSet<String>,
        known_commands: &'a HashSet<String>,
    ) -> BodyCompileInput<'a> {
        BodyCompileInput {
            body,
            skill_name: "test-skill",
            fragments: frags,
            vars: &cfg.vars,
            env: &cfg.env,
            known_skills,
            known_files,
            known_commands,
            tokenizer: &cfg.build.tokenizer,
        }
    }

    /// Empty `HashSet<String>` with a `'static` lifetime for test helpers.
    fn no_set() -> &'static HashSet<String> {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<HashSet<String>> = OnceLock::new();
        EMPTY.get_or_init(HashSet::new)
    }

    // ── plain body ─────────────────────────────────────────────────────────────

    #[test]
    fn plain_body_passes_through_unchanged() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "hello world",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "hello world");
    }

    // ── token pass ────────────────────────────────────────────────────────────

    #[test]
    fn token_count_is_nonzero_for_nonempty_body() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "hello world",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.tokens > 0, "expected at least one token");
    }

    #[test]
    fn empty_body_produces_zero_tokens() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert_eq!(out.tokens, 0);
    }

    // ── fragment pre-pass ─────────────────────────────────────────────────────

    #[test]
    fn known_fragment_is_interpolated() {
        let cfg = SkilletConfig::default();
        let mut raw = HashMap::new();
        let frags = render_fragments(&raw);
        raw.insert("footer".to_string(), "## Footer\nsome text\n".to_string());
        let frags = render_fragments(&raw);
        let out = compile_body(&default_input(
            "{> footer <}",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert!(out.text.contains("## Footer"));
        assert!(out.text.contains("some text"));
        assert_eq!(out.fragments_used, vec!["footer"]);
    }

    #[test]
    fn fragment_trailing_newline_is_stripped() {
        let cfg = SkilletConfig::default();
        let mut raw = HashMap::new();
        let frags = render_fragments(&raw);
        raw.insert("note".to_string(), "content\n\n".to_string());
        let frags = render_fragments(&raw);
        let out = compile_body(&default_input(
            "{> note <}",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "content");
    }

    #[test]
    fn unknown_fragment_produces_error_diagnostic() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "{> missing <}",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| { d.severity == DiagSeverity::Error && d.message.contains("missing") }));
    }

    #[test]
    fn fragment_used_only_once_in_fragments_used() {
        let cfg = SkilletConfig::default();
        let mut raw = HashMap::new();
        let frags = render_fragments(&raw);
        raw.insert("note".to_string(), "content\n".to_string());
        let frags = render_fragments(&raw);
        let body = "{> note <}\n{> note <}";
        let out = compile_body(&default_input(
            body,
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert_eq!(
            out.fragments_used.iter().filter(|f| *f == "note").count(),
            1
        );
    }

    #[test]
    fn nested_fragment_in_fragment_content_produces_error() {
        let cfg = SkilletConfig::default();
        let mut raw = HashMap::new();
        let frags = render_fragments(&raw);
        raw.insert("outer".to_string(), "text {> inner <} more\n".to_string());
        raw.insert("inner".to_string(), "inner content\n".to_string());
        let frags = render_fragments(&raw);
        let out = compile_body(&default_input(
            "{> outer <}",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        // Error must appear at the callsite line, not line 0, and must name
        // both the fragment being expanded and why it failed.
        let diag = out
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagSeverity::Error)
            .expect("expected an error diagnostic");
        assert!(
            diag.message.contains("outer"),
            "message should name the fragment: {}",
            diag.message
        );
        assert!(
            diag.message.contains("inner"),
            "message should name the nested include: {}",
            diag.message
        );
        assert_eq!(diag.line, 1, "error should be at callsite line 1");
    }

    // ── ref kinds ─────────────────────────────────────────────────────────────

    #[test]
    fn ref_directive_emits_backtick_wrapped_value() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`ref::foo.md`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "`foo.md`");
    }

    #[test]
    fn cmd_directive_emits_backtick_wrapped_value() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`cmd::git status`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "`git status`");
    }

    #[test]
    fn skill_directive_emits_backtick_wrapped_value() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`skill::other-skill`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "`other-skill`");
    }

    #[test]
    fn var_directive_substitutes_value_without_backticks() {
        let cfg = SkilletConfig::default(); // vars has project_name = "my-project"
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "deploy to `var::project_name`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "deploy to my-project");
    }

    #[test]
    fn unknown_var_produces_error_diagnostic() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`var::unknown`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| { d.severity == DiagSeverity::Error && d.message.contains("unknown") }));
    }

    #[test]
    fn env_directive_substitutes_value_without_backticks() {
        let cfg = SkilletConfig::default(); // env has CI = "false"
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "ci: `env::CI`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        let expected = std::env::var("CI").unwrap_or_else(|_| "false".to_string());
        assert_eq!(out.text, format!("ci: {}", expected));
    }

    #[test]
    fn unknown_env_produces_error_diagnostic() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`env::UNKNOWN`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| { d.severity == DiagSeverity::Error && d.message.contains("UNKNOWN") }));
    }

    #[test]
    fn agent_directive_emits_backtick_wrapped_value() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`agent::my-agent`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "`my-agent`");
    }

    #[test]
    fn url_directive_emits_backtick_wrapped_value() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "`url::https://example.com`",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "`https://example.com`");
    }

    // ── validation (non-empty known-sets activate checking) ───────────────────

    #[test]
    fn missing_ref_path_produces_error_when_known_files_nonempty() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let mut known_files = HashSet::new();
        known_files.insert("real.md".to_string());
        let input = BodyCompileInput {
            known_files: &known_files,
            ..default_input(
                "`ref::missing.md`",
                &frags,
                &cfg,
                no_set(),
                no_set(),
                no_set(),
            )
        };
        let out = compile_body(&input);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagSeverity::Error));
    }

    #[test]
    fn missing_skill_ref_produces_error_when_known_skills_nonempty() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let mut known_skills = HashSet::new();
        known_skills.insert("real-skill".to_string());
        let input = BodyCompileInput {
            known_skills: &known_skills,
            ..default_input("`skill::ghost`", &frags, &cfg, no_set(), no_set(), no_set())
        };
        let out = compile_body(&input);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagSeverity::Error));
    }

    #[test]
    fn missing_cmd_produces_warning_when_known_commands_nonempty() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let mut known_commands = HashSet::new();
        known_commands.insert("real-cmd".to_string());
        let input = BodyCompileInput {
            known_commands: &known_commands,
            ..default_input("`cmd::ghost`", &frags, &cfg, no_set(), no_set(), no_set())
        };
        let out = compile_body(&input);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagSeverity::Warning));
    }

    // ── escaped body ──────────────────────────────────────────────────────────

    #[test]
    fn escaped_body_collapses_double_backtick_to_single() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let out = compile_body(&default_input(
            "``skill::verbatim``",
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.text, "`skill::verbatim`");
    }

    // ── diagnostic line/col ───────────────────────────────────────────────────

    #[test]
    fn missing_fragment_diagnostic_has_nonzero_line() {
        let cfg = SkilletConfig::default();
        let frags = render_fragments(&HashMap::new());
        let body = "line one\n{> ghost <}";
        let out = compile_body(&default_input(
            body,
            &frags,
            &cfg,
            no_set(),
            no_set(),
            no_set(),
        ));
        let diag = out
            .diagnostics
            .iter()
            .find(|d| d.message.contains("ghost"))
            .expect("expected a diagnostic for 'ghost'");
        assert_eq!(diag.line, 2, "fragment is on line 2");
    }
}
