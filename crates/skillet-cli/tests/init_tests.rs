
mod common;

use std::fs;
use tempfile::TempDir;

#[test]
fn init_creates_skills_dir_fragments_dir_and_config() {
    // Arrange
    let tmp = TempDir::new().unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["init"]);

    // Assert — exit code
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert — filesystem layout
    assert!(tmp.path().join("src/skills").is_dir());
    assert!(tmp.path().join("src/skills/_fragments").is_dir());
    assert!(tmp.path().join("skills").is_dir());
    assert!(tmp.path().join("skillet.toml").is_file());

    // Assert — config values
    let content = fs::read_to_string(tmp.path().join("skillet.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&content).unwrap();

    assert_eq!(
        parsed["workspace"]["skills_src_dir"].as_str().unwrap(),
        "src/skills"
    );
    assert_eq!(
        parsed["workspace"]["skills_out_dir"].as_str().unwrap(),
        "skills"
    );
    assert_eq!(
        parsed["workspace"]["fragments_dir"].as_str().unwrap(),
        "src/skills/_fragments"
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
        "[workspace]\nskills_src_dir = 'src/skills'\nskills_out_dir = 'custom-skills'\nfragments_dir = 'src/skills/_fragments'\n",
    )
    .unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["init"]);

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
fn init_adopt_copies_skill_md_as_dot_pan_files() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let diagnose_out_dir = tmp.path().join("skills/diagnose");
    let caveman_out_dir = tmp.path().join("skills/caveman");
    fs::create_dir_all(&diagnose_out_dir).unwrap();
    fs::create_dir_all(&caveman_out_dir).unwrap();

    let diagnose_content = "---\nname: something-else\n---\n# Diagnose\n";
    let caveman_content = "# Caveman\n";
    fs::write(diagnose_out_dir.join("SKILL.md"), diagnose_content).unwrap();
    fs::write(caveman_out_dir.join("SKILL.md"), caveman_content).unwrap();

    // Act
    let out = common::run_skillet(tmp.path(), &["init", "--adopt"]);

    // Assert — command succeeds
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert — originals preserved
    assert!(diagnose_out_dir.join("SKILL.md").exists());
    assert!(caveman_out_dir.join("SKILL.md").exists());

    // Assert — .pan source files created under src/skills/
    let diagnose_pan = tmp.path().join("src/skills/diagnose/diagnose.pan");
    let caveman_pan = tmp.path().join("src/skills/caveman/caveman.pan");
    assert!(diagnose_pan.exists());
    assert!(caveman_pan.exists());

    assert_eq!(
        fs::read(diagnose_out_dir.join("SKILL.md")).unwrap(),
        fs::read(&diagnose_pan).unwrap()
    );
    assert_eq!(
        fs::read(caveman_out_dir.join("SKILL.md")).unwrap(),
        fs::read(&caveman_pan).unwrap()
    );

    // Assert — workspace scaffolded
    assert!(tmp.path().join("skillet.toml").is_file());
    assert!(tmp.path().join("src/skills/_fragments").is_dir());
}

// ── budget integration tests ──────────────────────────────────────────────────

#[test]
fn budget_runs_on_built_workspace() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    skillet::init::run(tmp.path(), false, false).unwrap();
    let skill_dir = tmp.path().join("src/skills/my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("my-skill.pan"),
        "---\nname: my-skill\ndescription: \"does stuff\"\n---\n\n## Usage\nrun it\n",
    )
    .unwrap();
    skillet::build::run(tmp.path(), None, &Default::default()).unwrap();

    // Act — should not error
    let result = skillet::budget::run(tmp.path(), None, skillet::budget::OutputFormat::Text);

    // Assert
    assert!(
        result.is_ok(),
        "budget::run should succeed on built workspace"
    );
}

#[test]
fn budget_json_format_produces_array() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    skillet::init::run(tmp.path(), false, false).unwrap();
    let skill_dir = tmp.path().join("src/skills/my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("my-skill.pan"),
        "---\nname: my-skill\ndescription: \"a skill\"\n---\n\n## Body\ncontent here\n",
    )
    .unwrap();
    skillet::build::run(tmp.path(), None, &Default::default()).unwrap();

    // Act
    let result = skillet::budget::run(tmp.path(), None, skillet::budget::OutputFormat::Json);

    // Assert
    assert!(result.is_ok());
}

#[test]
fn budget_single_skill_succeeds() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    skillet::init::run(tmp.path(), false, false).unwrap();
    let skill_dir = tmp.path().join("src/skills/alpha");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("alpha.pan"),
        "---\nname: alpha\ndescription: \"alpha skill\"\n---\n\n## Alpha\ncontent\n",
    )
    .unwrap();
    skillet::build::run(tmp.path(), Some("alpha"), &Default::default()).unwrap();

    // Act
    let result = skillet::budget::run(
        tmp.path(),
        Some("alpha"),
        skillet::budget::OutputFormat::Text,
    );

    // Assert
    assert!(result.is_ok());
}

#[test]
fn budget_errors_when_skill_not_built() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    skillet::init::run(tmp.path(), false, false).unwrap();
    let skill_dir = tmp.path().join("src/skills/unbuilt");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("unbuilt.pan"),
        "---\nname: unbuilt\ndescription: \"not built yet\"\n---\n\n## Body\n",
    )
    .unwrap();
    // deliberately skip skillet::build::run

    // Act
    let result = skillet::budget::run(
        tmp.path(),
        Some("unbuilt"),
        skillet::budget::OutputFormat::Text,
    );

    // Assert
    assert!(result.is_err(), "should error when SKILL.md is missing");
}
