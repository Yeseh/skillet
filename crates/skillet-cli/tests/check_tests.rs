
mod common;

use std::fs;
use tempfile::TempDir;

#[test]
fn check_passes_on_built_workspace() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    common::run_skillet(tmp.path(), &["build"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["check"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("up-to-date"), "stdout: {stdout}");
}

#[test]
fn check_exits_nonzero_when_lockfile_absent() {
    // Arrange — init workspace but never build
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["check"]);

    // Assert
    assert!(
        !out.status.success(),
        "check should fail when lockfile is absent"
    );
}

#[test]
fn check_exits_nonzero_when_source_modified_after_build() {
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
    let out = common::run_skillet(tmp.path(), &["check"]);

    // Assert
    assert!(
        !out.status.success(),
        "check should fail when source has changed"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stale"), "stdout: {stdout}");
}

#[test]
fn check_exits_nonzero_when_skill_deleted_after_build() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    common::run_skillet(tmp.path(), &["build"]);

    // Delete the skill directory without rebuilding
    fs::remove_dir_all(tmp.path().join("skills/my-skill")).unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["check"]);

    // Assert — lockfile still references my-skill → stale
    assert!(
        !out.status.success(),
        "check should fail when a built skill is deleted"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stale"), "stdout: {stdout}");
}

#[test]
fn check_json_format_produces_valid_json() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    common::run_skillet(tmp.path(), &["build"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["check", "--format", "json"]);

    // Assert
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");
    assert_eq!(parsed["fresh"], true);
    assert!(parsed["skills"].is_array());
}
