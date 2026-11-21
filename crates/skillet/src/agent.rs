//! Agent definitions: the skillet agent extension (version 1).
//!
//! An agent definition is a Markdown file with YAML frontmatter. The body is
//! the agent's system prompt. Agent Plugins v1 has no portable agent
//! component, so skillet publishes agents in its own reverse-domain extension
//! directory and adds the copies and manifest entries that each harness needs.
//!
//! Only `name` and `description` are interpreted. Every other frontmatter
//! field (for example `tools` or `model`) is written to the output without
//! change, and each harness decides what to do with it. Because the compiled
//! text is written as-is, field order and formatting are kept.

use gray_matter::{engine::YAML, Matter};
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;

use crate::compiler::compile::body_start_offset;

/// Reverse-domain namespace of the skillet agent extension.
pub const EXTENSION_NAMESPACE: &str = "io.github.yeseh.skillet";

/// Version of the agent extension format written by this skillet release.
pub const EXTENSION_VERSION: u32 = 1;

/// Namespace in which GitHub Copilot reads plugin agents.
pub const COPILOT_NAMESPACE: &str = "com.github.copilot";

/// File suffix of a compiled agent definition. Copilot requires it; Claude Code
/// and pi accept any `.md` file.
pub const OUTPUT_SUFFIX: &str = ".agent.md";

/// File suffix of an agent source.
pub const SOURCE_SUFFIX: &str = ".agent.pan";

/// Maximum body length in characters (the GitHub Copilot limit).
pub const MAX_BODY_CHARS: usize = 30_000;

/// Maximum agent name length.
pub const MAX_NAME_LEN: usize = 64;

/// Frontmatter fields that Claude Code ignores for plugin-provided agents.
pub const CLAUDE_IGNORED_FIELDS: &[&str] =
    &["hooks", "mcpServers", "permissionMode", "initialPrompt"];

/// Path of the canonical agent file, relative to the plugin output directory.
pub fn canonical_path(name: &str) -> String {
    format!("{EXTENSION_NAMESPACE}/agents/{name}{OUTPUT_SUFFIX}")
}

/// Path of the GitHub Copilot copy, relative to the plugin output directory.
pub fn copilot_path(name: &str) -> String {
    format!("{COPILOT_NAMESPACE}/agents/{name}{OUTPUT_SUFFIX}")
}

// ── Name ───────────────────────────────────────────────────────────────────────

/// A validated agent name.
///
/// 1–64 characters of `a-z`, `0-9` and `-`, starting and ending with a letter
/// or digit, without `--`. Claude Code, GitHub Copilot and pi all accept names
/// that follow this rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentName(String);

/// Why a string is not a valid [`AgentName`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentNameError {
    /// The name is empty.
    Empty,
    /// The name is longer than [`MAX_NAME_LEN`].
    TooLong,
    /// The name contains a character outside `a-z`, `0-9` and `-`.
    InvalidChar(char),
    /// The name starts or ends with `-`.
    BadBoundary,
    /// The name contains `--`.
    DoubleHyphen,
}

impl AgentNameError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too-long",
            Self::InvalidChar(_) => "invalid-char",
            Self::BadBoundary => "bad-boundary",
            Self::DoubleHyphen => "double-hyphen",
        }
    }
}

impl fmt::Display for AgentNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "name is empty"),
            Self::TooLong => write!(f, "name is longer than {MAX_NAME_LEN} characters"),
            Self::InvalidChar(c) => {
                write!(f, "name contains '{c}'; use only a-z, 0-9 and '-'")
            }
            Self::BadBoundary => write!(f, "name must start and end with a letter or digit"),
            Self::DoubleHyphen => write!(f, "name must not contain '--'"),
        }
    }
}

impl AgentName {
    /// Validates `value` against the portable agent name rule.
    pub fn parse(value: &str) -> Result<Self, AgentNameError> {
        if value.is_empty() {
            return Err(AgentNameError::Empty);
        }
        if value.len() > MAX_NAME_LEN {
            return Err(AgentNameError::TooLong);
        }
        if let Some(c) = value
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            return Err(AgentNameError::InvalidChar(c));
        }
        if value.starts_with('-') || value.ends_with('-') {
            return Err(AgentNameError::BadBoundary);
        }
        if value.contains("--") {
            return Err(AgentNameError::DoubleHyphen);
        }
        Ok(Self(value.to_string()))
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Definition ─────────────────────────────────────────────────────────────────

/// Why a compiled agent is not a valid agent definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    /// The text has no `---` frontmatter block.
    MissingFrontmatter,
    /// The frontmatter is not valid YAML, or `name`/`description` is not a string.
    InvalidFrontmatter(String),
    /// The frontmatter has no `name`.
    MissingName,
    /// The `name` does not follow the agent name rule.
    InvalidName(AgentNameError),
    /// The `name` is not equal to the source file name.
    NameMismatch {
        /// Value of the `name` field.
        name: String,
        /// Name taken from the source file.
        file_name: String,
    },
    /// The frontmatter has no `description`, or it is empty.
    MissingDescription,
    /// The body is longer than [`MAX_BODY_CHARS`].
    BodyTooLong {
        /// Body length in characters.
        chars: usize,
    },
}

impl AgentError {
    /// Stable machine-readable code. The lint rule for an error is `agent-{code}`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingFrontmatter => "missing-frontmatter",
            Self::InvalidFrontmatter(_) => "invalid-frontmatter",
            Self::MissingName => "missing-name",
            Self::InvalidName(_) => "invalid-name",
            Self::NameMismatch { .. } => "name-mismatch",
            Self::MissingDescription => "missing-description",
            Self::BodyTooLong { .. } => "body-too-long",
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFrontmatter => {
                write!(
                    f,
                    "agent has no frontmatter; add a '---' block with name and description"
                )
            }
            Self::InvalidFrontmatter(reason) => write!(f, "invalid frontmatter: {reason}"),
            Self::MissingName => write!(f, "frontmatter has no 'name'"),
            Self::InvalidName(e) => write!(f, "invalid agent name: {e}"),
            Self::NameMismatch { name, file_name } => write!(
                f,
                "name '{name}' must equal the file name '{file_name}' \
                 (GitHub Copilot uses the file name as the agent id)"
            ),
            Self::MissingDescription => write!(f, "frontmatter has no 'description'"),
            Self::BodyTooLong { chars } => write!(
                f,
                "body has {chars} characters; GitHub Copilot accepts at most {MAX_BODY_CHARS}"
            ),
        }
    }
}

impl std::error::Error for AgentError {}

/// The parts of an agent definition that skillet validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    /// Validated agent name.
    pub name: AgentName,
    /// Agent description.
    pub description: String,
    /// Names of the pass-through frontmatter fields, sorted.
    pub other_fields: Vec<String>,
}

#[derive(Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, IgnoredAny>,
}

impl AgentDefinition {
    /// Validates compiled agent text.
    ///
    /// `file_name` is the agent name taken from the source file
    /// (`{file_name}.agent.pan`).
    pub fn parse(compiled: &str, file_name: &str) -> Result<Self, AgentError> {
        let text = compiled.strip_prefix('\u{feff}').unwrap_or(compiled);

        let fm: RawFrontmatter = Matter::<YAML>::new()
            .parse::<RawFrontmatter>(text)
            .map_err(|e| AgentError::InvalidFrontmatter(e.to_string()))?
            .data
            .ok_or(AgentError::MissingFrontmatter)?;

        let raw_name = fm.name.ok_or(AgentError::MissingName)?;
        let name = AgentName::parse(&raw_name).map_err(AgentError::InvalidName)?;
        if name.as_str() != file_name {
            return Err(AgentError::NameMismatch {
                name: raw_name,
                file_name: file_name.to_string(),
            });
        }

        let description = fm
            .description
            .filter(|d| !d.trim().is_empty())
            .ok_or(AgentError::MissingDescription)?;

        let chars = text[body_start_offset(text)..].chars().count();
        if chars > MAX_BODY_CHARS {
            return Err(AgentError::BodyTooLong { chars });
        }

        Ok(Self {
            name,
            description,
            other_fields: fm.other.into_keys().collect(),
        })
    }

    /// Pass-through fields that Claude Code ignores for plugin agents.
    pub fn claude_ignored_fields(&self) -> Vec<&str> {
        self.other_fields
            .iter()
            .map(String::as_str)
            .filter(|f| CLAUDE_IGNORED_FIELDS.contains(f))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(frontmatter: &str, body: &str) -> String {
        format!("---\n{frontmatter}\n---\n{body}")
    }

    #[test]
    fn name_accepts_portable_names() {
        for ok in ["a", "reviewer", "code-reviewer-2", "x".repeat(64).as_str()] {
            assert!(AgentName::parse(ok).is_ok(), "{ok} should be valid");
        }
    }

    #[test]
    fn name_rejects_each_rule_with_its_code() {
        let cases = [
            ("", "empty"),
            (&*"x".repeat(65), "too-long"),
            ("Reviewer", "invalid-char"),
            ("my:agent", "invalid-char"),
            ("my_agent", "invalid-char"),
            ("-start", "bad-boundary"),
            ("end-", "bad-boundary"),
            ("has--double", "double-hyphen"),
        ];
        for (input, code) in cases {
            let err = AgentName::parse(input).unwrap_err();
            assert_eq!(err.code(), code, "input {input:?}");
        }
    }

    #[test]
    fn parse_accepts_valid_agent_and_lists_pass_through_fields() {
        // Arrange
        let text = agent(
            "name: reviewer\ndescription: Reviews code\ntools: [Read, read]\nmodel: sonnet",
            "You review code.\n",
        );

        // Act
        let def = AgentDefinition::parse(&text, "reviewer").unwrap();

        // Assert
        assert_eq!(def.name.as_str(), "reviewer");
        assert_eq!(def.description, "Reviews code");
        assert_eq!(def.other_fields, vec!["model", "tools"]);
        assert!(def.claude_ignored_fields().is_empty());
    }

    #[test]
    fn parse_reports_claude_ignored_fields() {
        let text = agent(
            "name: a\ndescription: d\npermissionMode: plan\nhooks: {}",
            "",
        );
        let def = AgentDefinition::parse(&text, "a").unwrap();
        assert_eq!(def.claude_ignored_fields(), vec!["hooks", "permissionMode"]);
    }

    #[test]
    fn parse_errors_have_stable_codes() {
        let long_body = "x".repeat(MAX_BODY_CHARS + 1);
        let cases: Vec<(String, &str, &str)> = vec![
            ("no frontmatter".into(), "a", "missing-frontmatter"),
            (agent("name: [a", ""), "a", "invalid-frontmatter"),
            (
                agent("name: 5\ndescription: d", ""),
                "a",
                "invalid-frontmatter",
            ),
            (agent("description: d", ""), "a", "missing-name"),
            (agent("name: A\ndescription: d", ""), "a", "invalid-name"),
            (agent("name: b\ndescription: d", ""), "a", "name-mismatch"),
            (agent("name: a", ""), "a", "missing-description"),
            (
                agent("name: a\ndescription: '  '", ""),
                "a",
                "missing-description",
            ),
            (
                agent("name: a\ndescription: d", &long_body),
                "a",
                "body-too-long",
            ),
        ];
        for (text, file_name, code) in cases {
            let err = AgentDefinition::parse(&text, file_name).unwrap_err();
            assert_eq!(err.code(), code, "text {text:?}");
        }
    }

    #[test]
    fn body_limit_counts_characters_not_bytes() {
        let body = "é".repeat(MAX_BODY_CHARS);
        let text = agent("name: a\ndescription: d", &body);
        assert!(AgentDefinition::parse(&text, "a").is_ok());
    }

    #[test]
    fn output_paths_use_extension_namespaces() {
        assert_eq!(
            canonical_path("reviewer"),
            "io.github.yeseh.skillet/agents/reviewer.agent.md"
        );
        assert_eq!(
            copilot_path("reviewer"),
            "com.github.copilot/agents/reviewer.agent.md"
        );
    }
}
