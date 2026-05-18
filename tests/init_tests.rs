#![allow(missing_docs)]

use std::fs;
use tempfile::TempDir;

fn skillet_bin() -> std::path::PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if path.ends_with("deps") {
        path = path.parent().unwrap().to_path_buf();
    }
    path.join("skillet")
}

fn run_skillet(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(skillet_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run skillet")
}

#[test]
fn init_creates_skills_dir_fragments_dir_and_config() {
    // Arrange
    let tmp = TempDir::new().unwrap();

    // Act
    let out = run_skillet(tmp.path(), &["init"]);

    // Assert — exit code
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert — filesystem layout
    assert!(tmp.path().join("skills").is_dir());
    assert!(tmp.path().join("skills/_fragments").is_dir());
    assert!(tmp.path().join("skillet.toml").is_file());

    // Assert — config values
    let content = fs::read_to_string(tmp.path().join("skillet.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();

    assert_eq!(
        parsed["workspace"]["skills_dir"].as_str().unwrap(),
        "skills"
    );
    assert_eq!(
        parsed["workspace"]["fragments_dir"].as_str().unwrap(),
        "skills/_fragments"
    );
    assert_eq!(
        parsed["lint"]["max_activation_tokens"]
            .as_integer()
            .unwrap(),
        4000
    );
    assert_eq!(
        parsed["lint"]["max_discovery_tokens"].as_integer().unwrap(),
        100
    );
    assert_eq!(
        parsed["lint"]["max_fragment_tokens"].as_integer().unwrap(),
        500
    );
    assert_eq!(
        parsed["build"]["tokenizer"].as_str().unwrap(),
        "cl100k_base"
    );
    assert!(!parsed["build"]["verify_urls"].as_bool().unwrap());
    assert_eq!(
        parsed["vars"]["project_name"].as_str().unwrap(),
        "my-project"
    );
    assert_eq!(parsed["env"]["CI"]["default"].as_str().unwrap(), "false");
    assert_eq!(
        parsed["env"]["TEAM_NAME"]["default"].as_str().unwrap(),
        "engineering"
    );
}

#[test]
fn init_refuses_to_overwrite_existing_skillet_toml() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("skillet.toml"),
        "[workspace]\nskills_dir = 'custom-skills'\nfragments_dir = 'custom-skills/_fragments'\n",
    )
    .unwrap();

    // Act
    let out = run_skillet(tmp.path(), &["init"]);

    // Assert — command fails
    assert!(
        !out.status.success(),
        "expected failure when skillet.toml exists"
    );

    // Assert — original content is preserved
    let content = fs::read_to_string(tmp.path().join("skillet.toml")).unwrap();
    assert!(
        content.contains("custom-skills"),
        "original content should be preserved"
    );
}

#[test]
fn init_adopt_copies_skill_md_as_dot_skill_files() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let diagnose_dir = tmp.path().join("skills/diagnose");
    let caveman_dir = tmp.path().join("skills/caveman");
    fs::create_dir_all(&diagnose_dir).unwrap();
    fs::create_dir_all(&caveman_dir).unwrap();

    let diagnose_content = "---\nname: something-else\n---\n# Diagnose\n";
    let caveman_content = "# Caveman\n";
    fs::write(diagnose_dir.join("SKILL.md"), diagnose_content).unwrap();
    fs::write(caveman_dir.join("SKILL.md"), caveman_content).unwrap();

    // Act
    let out = run_skillet(tmp.path(), &["init", "--adopt"]);

    // Assert — command succeeds
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert — originals preserved
    assert!(diagnose_dir.join("SKILL.md").exists());
    assert!(caveman_dir.join("SKILL.md").exists());

    // Assert — .skill files created with matching content
    assert!(diagnose_dir.join("diagnose.skill").exists());
    assert!(caveman_dir.join("caveman.skill").exists());

    assert_eq!(
        fs::read(diagnose_dir.join("SKILL.md")).unwrap(),
        fs::read(diagnose_dir.join("diagnose.skill")).unwrap()
    );
    assert_eq!(
        fs::read(caveman_dir.join("SKILL.md")).unwrap(),
        fs::read(caveman_dir.join("caveman.skill")).unwrap()
    );

    // Assert — workspace scaffolded
    assert!(tmp.path().join("skillet.toml").is_file());
    assert!(tmp.path().join("skills/_fragments").is_dir());
}

#[test]
fn new_creates_skill_directory_and_source_file_with_expected_frontmatter() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let out = run_skillet(tmp.path(), &["init"]);
    assert!(out.status.success());

    // Act
    let out = run_skillet(tmp.path(), &["new", "my-skill"]);

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
        content.contains("description: \"\""),
        "should contain empty description"
    );
    assert!(content.contains("# my-skill"), "should contain heading");
}

#[test]
fn new_refuses_to_overwrite_existing_skill_directory() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    run_skillet(tmp.path(), &["init"]);
    let out = run_skillet(tmp.path(), &["new", "dupe"]);
    assert!(out.status.success(), "first new should succeed");

    // Act
    let out = run_skillet(tmp.path(), &["new", "dupe"]);

    // Assert
    assert!(!out.status.success(), "second new should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("dupe") || stderr.contains("already exists"),
        "error should mention conflict"
    );
}
