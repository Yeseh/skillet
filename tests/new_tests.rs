#![allow(missing_docs)]

mod common;

use std::fs;
use tempfile::TempDir;

#[test]
fn new_creates_skill_directory_and_source_file_with_expected_frontmatter() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let out = common::run_skillet(tmp.path(), &["init"]);
    assert!(out.status.success());

    // Act
    let out = common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let skill_file = tmp.path().join("skills/my-skill/my-skill.skill");
    assert!(skill_file.exists(), "skill file should exist");
    let content = fs::read_to_string(&skill_file).unwrap();
    assert!(content.contains("name: my-skill"), "should contain name");
    assert!(
        content.contains("description:"),
        "should contain empty description"
    );
    assert!(content.contains("# my-skill"), "should contain heading");
}

#[test]
fn new_refuses_to_overwrite_existing_skill_directory() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    let out = common::run_skillet(tmp.path(), &["new", "dupe"]);
    assert!(out.status.success(), "first new should succeed");

    // Act
    let out = common::run_skillet(tmp.path(), &["new", "dupe"]);

    // Assert
    assert!(!out.status.success(), "second new should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("dupe") || stderr.contains("already exists"),
        "error should mention conflict"
    );
}
