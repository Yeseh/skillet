mod common;

use std::fs;
use std::path::Path;
use tempfile::TempDir;

const CONFIG: &str = r#"[workspace]

[workspace.publish]
agents = ["claude", "github-copilot"]
marketplace_name = "test"
owner_name = "Test"

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"
verify_urls = false

[plugin.tools]
src_dir = "src/skills"
out_dir = "out"
version = "1.0.0"
publish = true
agents_dir = "src/agents"
"#;

const REVIEWER: &str = "---
name: reviewer
description: Reviews code changes
tools: [Read, Grep, read, grep]
model: sonnet
color: blue
---

You review code.
";

fn workspace_with_agent(file_name: &str, content: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("skillet.toml"), CONFIG).unwrap();
    fs::create_dir_all(tmp.path().join("src/skills")).unwrap();
    let agents = tmp.path().join("src/agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join(file_name), content).unwrap();
    tmp
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn build_writes_canonical_and_copilot_agent_with_pass_through_fields() {
    // Arrange
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);

    // Act
    let out = common::run_skillet(tmp.path(), &["build"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let canonical = read(
        tmp.path(),
        "out/io.github.yeseh.skillet/agents/reviewer.agent.md",
    );
    let copilot = read(
        tmp.path(),
        "out/com.github.copilot/agents/reviewer.agent.md",
    );
    assert_eq!(
        canonical, REVIEWER,
        "frontmatter and body pass through unchanged"
    );
    assert_eq!(copilot, canonical);

    let lock = read(tmp.path(), "skillet.lock");
    assert!(lock.contains("[agents.reviewer]"), "lockfile: {lock}");
}

#[test]
fn build_fails_with_error_code_when_name_does_not_match_file() {
    // Arrange
    let tmp = workspace_with_agent("checker.agent.pan", REVIEWER);

    // Act
    let out = common::run_skillet(tmp.path(), &["build"]);

    // Assert
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("[agent-name-mismatch]"), "stderr: {stderr}");
}

#[test]
fn build_fails_on_ref_directive_in_agent() {
    let src = "---\nname: a\ndescription: d\n---\n\nRead `ref::notes`.\n";
    let tmp = workspace_with_agent("a.agent.pan", src);

    let out = common::run_skillet(tmp.path(), &["build"]);

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("check errors in agent 'a'"),
        "stderr: {stderr}"
    );
}

#[test]
fn publish_declares_extension_and_lists_agents_for_claude() {
    // Arrange
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);

    // Act
    let out = common::run_skillet(tmp.path(), &["publish"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let plugin: serde_json::Value =
        serde_json::from_str(&read(tmp.path(), "out/plugin.json")).unwrap();
    assert_eq!(
        plugin["extensions"]["io.github.yeseh.skillet"]["version"],
        1
    );
    assert!(
        plugin.get("agents").is_none(),
        "root manifest has no agents field"
    );

    let claude: serde_json::Value =
        serde_json::from_str(&read(tmp.path(), "out/.claude-plugin/plugin.json")).unwrap();
    assert_eq!(claude["name"], "tools");
    assert_eq!(
        claude["agents"],
        serde_json::json!(["./io.github.yeseh.skillet/agents/reviewer.agent.md"])
    );
    assert!(claude.get("extensions").is_none());
}

#[test]
fn publish_without_agents_writes_no_extension_or_claude_manifest() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("skillet.toml"), CONFIG).unwrap();
    fs::create_dir_all(tmp.path().join("src/skills")).unwrap();
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["publish"]);

    // Assert
    assert!(out.status.success());
    let plugin = read(tmp.path(), "out/plugin.json");
    assert!(!plugin.contains("extensions"), "plugin.json: {plugin}");
    assert!(!tmp.path().join("out/.claude-plugin").exists());
}

#[test]
fn lint_reports_agent_rules() {
    // Arrange
    let src = "---\nname: a\ndescription: d\npermissionMode: plan\n---\n\nBody.\n";
    let tmp = workspace_with_agent("a.agent.pan", src);

    // Act
    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);

    // Assert
    let stdout = String::from_utf8_lossy(&out.stdout);
    let diags: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    assert!(
        diags
            .iter()
            .any(|d| d["rule"] == "agent-field-ignored-by-claude" && d["skill"] == "a"),
        "diagnostics: {stdout}"
    );
}

#[test]
fn lint_reports_invalid_agent_definition_as_error() {
    let src = "---\nname: a\n---\n\nBody.\n";
    let tmp = workspace_with_agent("a.agent.pan", src);

    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);

    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("agent-missing-description"), "{stdout}");
}

fn json(out: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"))
}

#[test]
fn check_reports_built_agent_as_fresh() {
    // Arrange
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);
    common::run_skillet(tmp.path(), &["build"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["check", "--format", "json"]);

    // Assert
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let report = json(&out);
    assert_eq!(report["fresh"], true);
    assert_eq!(report["agents"][0]["name"], "reviewer");
    assert_eq!(report["agents"][0]["fresh"], true);
}

#[test]
fn check_detects_edited_copilot_copy() {
    // Arrange
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);
    common::run_skillet(tmp.path(), &["build"]);
    fs::write(
        tmp.path()
            .join("out/com.github.copilot/agents/reviewer.agent.md"),
        "edited",
    )
    .unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["check", "--format", "json"]);

    // Assert
    assert!(!out.status.success());
    let diffs = &json(&out)["agents"][0]["diffs"];
    assert_eq!(diffs[0]["kind"], "agent_output_changed", "{diffs}");
}

#[test]
fn check_detects_changed_and_removed_agent_sources() {
    // Arrange
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);
    common::run_skillet(tmp.path(), &["build"]);
    let agents = tmp.path().join("src/agents");
    fs::write(
        agents.join("reviewer.agent.pan"),
        REVIEWER.replace("You", "We"),
    )
    .unwrap();

    // Act
    let changed = common::run_skillet(tmp.path(), &["check"]);
    fs::remove_file(agents.join("reviewer.agent.pan")).unwrap();
    let removed = common::run_skillet(tmp.path(), &["check", "--format", "json"]);

    // Assert
    assert!(!changed.status.success());
    let text = String::from_utf8_lossy(&changed.stdout);
    assert!(text.contains("✗ agent reviewer: source"), "{text}");
    assert!(text.contains("0 skills and 1 agent stale"), "{text}");

    assert!(!removed.status.success());
    assert_eq!(
        json(&removed)["agents"][0]["diffs"][0]["kind"],
        "source_missing"
    );
}

#[test]
fn lint_reports_stale_agent_build() {
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);
    common::run_skillet(tmp.path(), &["build"]);
    fs::remove_file(
        tmp.path()
            .join("out/io.github.yeseh.skillet/agents/reviewer.agent.md"),
    )
    .unwrap();

    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);

    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stale-build"), "{stdout}");
}

#[test]
fn budget_reports_agents_and_includes_them_in_totals() {
    // Arrange
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);
    common::run_skillet(tmp.path(), &["build"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["budget", "--format", "json"]);
    let text = common::run_skillet(tmp.path(), &["budget"]);

    // Assert
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = json(&out);
    let row = &report["agents"][0];
    assert_eq!(row["name"], "reviewer");
    assert!(row["activation"].as_u64().unwrap() > 0);
    assert_eq!(
        row["transitive"], row["activation"],
        "no fragments, no references"
    );
    assert_eq!(report["totals"]["activation"], row["activation"]);

    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("Agent") && text.contains("reviewer"),
        "{text}"
    );
}

#[test]
fn budget_reads_agent_output_when_lockfile_is_missing() {
    let tmp = workspace_with_agent("reviewer.agent.pan", REVIEWER);
    common::run_skillet(tmp.path(), &["build"]);
    fs::remove_file(tmp.path().join("skillet.lock")).unwrap();

    let out = common::run_skillet(tmp.path(), &["budget", "--format", "json"]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let row = &json(&out)["agents"][0];
    assert!(row["discovery"].as_u64().unwrap() > 0);
    assert_eq!(row["transitive"], row["activation"]);
}
