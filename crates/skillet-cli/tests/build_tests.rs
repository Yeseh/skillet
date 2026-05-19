
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

#[test]
fn build_errors_on_nested_fragment_include() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Create a fragment whose content itself contains an include directive
    fs::write(
        tmp.path().join("src/skills/_fragments/inner.fragment.pan"),
        "## Inner\ninner content\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("src/skills/_fragments/outer.fragment.pan"),
        "## Outer\n{{> inner }}\n",
    )
    .unwrap();

    // Append a reference to the outer fragment in the skill source
    let source_path = tmp.path().join("src/skills/my-skill/my-skill.pan");
    let mut source = fs::read_to_string(&source_path).unwrap();
    source.push_str("\n{{> outer }}\n");
    fs::write(&source_path, &source).unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["build", "my-skill"]);

    // Assert — build must fail with a message about nesting
    assert!(
        !out.status.success(),
        "build should fail on nested fragment include"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nested"),
        "stderr should mention 'nested': {stderr}"
    );
}

#[test]
fn build_records_token_counts_in_lockfile() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    // Act
    let out = common::run_skillet(tmp.path(), &["build"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert — lockfile should contain token fields
    let lock = fs::read_to_string(tmp.path().join("skillet.lock")).unwrap();
    assert!(
        lock.contains("activation_tokens"),
        "lockfile should contain activation_tokens"
    );
    assert!(
        lock.contains("discovery_tokens"),
        "lockfile should contain discovery_tokens"
    );
}
#[test]
fn build_records_fragment_hash_in_lockfile() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);

    fs::write(
        tmp.path().join("src/skills/_fragments/note.fragment.pan"),
        "## Shared Note\ncontent\n",
    )
    .unwrap();
    let source_path = tmp.path().join("src/skills/my-skill/my-skill.pan");
    let mut source = fs::read_to_string(&source_path).unwrap();
    source.push_str("\n{{> note }}\n");
    fs::write(&source_path, &source).unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["build", "my-skill"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert — lockfile should contain a [fragments.note] section with a hash and tokens
    let lock = fs::read_to_string(tmp.path().join("skillet.lock")).unwrap();
    assert!(
        lock.contains("note"),
        "lockfile should reference the 'note' fragment"
    );
    assert!(
        lock.contains("sha256:"),
        "lockfile should contain a sha256 hash for the fragment"
    );
    assert!(
        lock.contains("my-skill"),
        "lockfile fragments section should list 'my-skill' in used_by"
    );
    assert!(
        lock.contains("tokens"),
        "lockfile fragment entry should contain a tokens field"
    );
}
