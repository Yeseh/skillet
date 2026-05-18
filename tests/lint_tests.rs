#![allow(missing_docs)]

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
    assert!(!out.status.success(), "should fail because SKILL.md is stale");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stale-build"));
}
