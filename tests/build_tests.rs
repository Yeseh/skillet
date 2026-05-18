#![allow(missing_docs)]

mod common;

use std::fs;
use tempfile::TempDir;

#[test]
fn build_compiles_skill_source_to_skill_md() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["build"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let skill_md = tmp.path().join("skills/my-skill/SKILL.md");
    assert!(skill_md.exists(), "SKILL.md should be written");
    let content = fs::read_to_string(&skill_md).unwrap();
    assert!(content.starts_with("---\n"), "should have frontmatter");
    assert!(content.contains("my-skill"), "should contain skill name");
}

#[test]
fn build_updates_skillet_lock() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Act
    common::run_skillet(tmp.path(), &["build"]);

    // Assert
    let lock = tmp.path().join("skillet.lock");
    assert!(lock.exists(), "skillet.lock should be written");
    let content = fs::read_to_string(&lock).unwrap();
    assert!(content.contains("# Auto-generated"), "should have header");
    assert!(content.contains("my-skill"), "should reference the skill");
}

#[test]
fn build_name_compiles_only_that_skill() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "skill-a"]);
    common::run_skillet(tmp.path(), &["new", "skill-b"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["build", "skill-a"]);

    // Assert
    assert!(out.status.success());
    assert!(tmp.path().join("skills/skill-a/SKILL.md").exists());
    assert!(!tmp.path().join("skills/skill-b/SKILL.md").exists());
}

#[test]
fn build_expands_fragment_includes_in_output() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    fs::write(
        tmp.path().join("src/skills/_fragments/note.fragment.pan"),
        "## Shared Note\nfragment content here\n",
    )
    .unwrap();
    // Append fragment include to the skill source
    let source_path = tmp.path().join("src/skills/my-skill/my-skill.pan");
    let mut source = fs::read_to_string(&source_path).unwrap();
    source.push_str("\n{{> note }}\n");
    fs::write(&source_path, &source).unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["build", "my-skill"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output = fs::read_to_string(tmp.path().join("skills/my-skill/SKILL.md")).unwrap();
    assert!(output.contains("## Shared Note"));
    assert!(output.contains("fragment content here"));
    assert!(!output.contains("{{> note }}"));
}
