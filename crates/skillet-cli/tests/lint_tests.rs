mod common;

use std::fs;
use tempfile::TempDir;

#[test]
fn lint_passes_on_built_workspace() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    common::run_skillet(tmp.path(), &["build"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["lint"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("no issues found"));
}

#[test]
fn lint_exits_nonzero_on_errors() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    // Create a skill with a wrong name in frontmatter — does not build
    let skill_dir = tmp.path().join("src/skills/my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("my-skill.pan"),
        "---\nname: wrong-name\ndescription: x\n---\n\n# body\n",
    )
    .unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["lint"]);

    // Assert
    assert!(!out.status.success(), "lint should exit non-zero on errors");
}

#[test]
fn lint_json_format_produces_array() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    common::run_skillet(tmp.path(), &["build"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);

    // Assert
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Empty diagnostics → JSON empty array
    assert!(
        stdout.trim().starts_with('['),
        "expected JSON array, got: {stdout}"
    );
}

#[test]
fn lint_stale_build_fires_after_source_edit() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    common::run_skillet(tmp.path(), &["build"]);

    // Mutate the source after building
    let source = tmp.path().join("src/skills/my-skill/my-skill.pan");
    let mut content = fs::read_to_string(&source).unwrap();
    content.push_str("\nExtra line added after build.\n");
    fs::write(&source, &content).unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["lint"]);

    // Assert
    assert!(
        !out.status.success(),
        "should fail because SKILL.md is stale"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stale-build"));
}

const SHARED_PASSAGE: &str = "First sentence here. Second sentence here. Third sentence here.";

fn write_skill(dir: &std::path::Path, name: &str, body: &str) {
    // Write a minimal built skill: both source and SKILL.md output.
    let src_dir = dir.join("src/skills").join(name);
    fs::create_dir_all(&src_dir).unwrap();
    let description = "a test skill";
    let pan = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
    fs::write(src_dir.join(format!("{name}.pan")), &pan).unwrap();
    // Write the compiled output manually so stale-build doesn't interfere.
    let out_dir = dir.join("skills").join(name);
    fs::create_dir_all(&out_dir).unwrap();
    // Build it properly via the CLI so the lockfile/hash is consistent.
    common::run_skillet(dir, &["build", name]);
}

#[test]
fn lint_duplication_warns_on_cross_skill_shared_passage() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    write_skill(tmp.path(), "alpha", SHARED_PASSAGE);
    write_skill(tmp.path(), "beta", SHARED_PASSAGE);

    // Act
    let out = common::run_skillet(tmp.path(), &["lint"]);

    // Assert — stale-build may fire; what matters is duplication warning present
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("duplication"),
        "expected duplication warning in output: {stdout}"
    );
    assert!(
        stdout.contains("fragment"),
        "expected extract-to-fragment guidance in output: {stdout}"
    );
}

#[test]
fn lint_duplication_json_includes_structured_fields() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    write_skill(tmp.path(), "alpha", SHARED_PASSAGE);
    write_skill(tmp.path(), "beta", SHARED_PASSAGE);

    // Act — run with JSON output; ignore exit code (may be non-zero due to stale-build)
    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Assert — output must be valid JSON array
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");
    let arr = parsed.as_array().expect("expected JSON array");

    let dup = arr
        .iter()
        .find(|d| d["rule"] == "duplication")
        .expect("expected a duplication entry");
    assert!(
        dup["duplicated_text"].is_string(),
        "duplicated_text should be a string"
    );
    let skills = dup["affected_skills"]
        .as_array()
        .expect("affected_skills should be an array");
    assert!(skills.len() >= 2, "expected at least 2 affected skills");
    let skill_names: Vec<&str> = skills.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(skill_names.contains(&"alpha"));
    assert!(skill_names.contains(&"beta"));
}

#[test]
fn lint_no_duplication_warning_for_unique_skills() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    write_skill(
        tmp.path(),
        "alpha",
        "Unique content only in alpha. Nothing shared. Completely different.",
    );
    write_skill(
        tmp.path(),
        "beta",
        "Separate content only in beta. Nothing shared. Completely different.",
    );

    // Act
    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let arr = parsed.as_array().expect("array");

    // Assert — no duplication finding
    assert!(
        !arr.iter().any(|d| d["rule"] == "duplication"),
        "unexpected duplication warning for unique skills"
    );
}
