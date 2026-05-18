use std::fs;
use tempfile::TempDir;

fn skillet_bin() -> std::path::PathBuf {
    // Use cargo to get the binary
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    // When running tests, the exe is in deps/, go up one
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
fn test_init_happy_path() {
    let tmp = TempDir::new().unwrap();
    let out = run_skillet(tmp.path(), &["init"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(tmp.path().join("skills").is_dir());
    assert!(tmp.path().join("skills/_fragments").is_dir());
    assert!(tmp.path().join("skillet.toml").is_file());

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
fn test_init_no_overwrite() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("skillet.toml"),
        "[workspace]\nskills_dir = 'custom-skills'\nfragments_dir = 'custom-skills/_fragments'\n",
    )
    .unwrap();

    let out = run_skillet(tmp.path(), &["init"]);
    assert!(
        !out.status.success(),
        "expected failure when skillet.toml exists"
    );

    // Original content preserved
    let content = fs::read_to_string(tmp.path().join("skillet.toml")).unwrap();
    assert!(
        content.contains("custom-skills"),
        "original content should be preserved"
    );
}

#[test]
fn test_init_adopt() {
    let tmp = TempDir::new().unwrap();
    let diagnose_dir = tmp.path().join("skills/diagnose");
    let caveman_dir = tmp.path().join("skills/caveman");
    fs::create_dir_all(&diagnose_dir).unwrap();
    fs::create_dir_all(&caveman_dir).unwrap();

    let diagnose_content = "---\nname: something-else\n---\n# Diagnose\n";
    let caveman_content = "# Caveman\n";
    fs::write(diagnose_dir.join("SKILL.md"), diagnose_content).unwrap();
    fs::write(caveman_dir.join("SKILL.md"), caveman_content).unwrap();

    let out = run_skillet(tmp.path(), &["init", "--adopt"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Originals preserved
    assert!(diagnose_dir.join("SKILL.md").exists());
    assert!(caveman_dir.join("SKILL.md").exists());

    // .skill files created
    assert!(diagnose_dir.join("diagnose.skill").exists());
    assert!(caveman_dir.join("caveman.skill").exists());

    // Content matches byte-for-byte
    let d_orig = fs::read(diagnose_dir.join("SKILL.md")).unwrap();
    let d_copy = fs::read(diagnose_dir.join("diagnose.skill")).unwrap();
    assert_eq!(d_orig, d_copy);

    let c_orig = fs::read(caveman_dir.join("SKILL.md")).unwrap();
    let c_copy = fs::read(caveman_dir.join("caveman.skill")).unwrap();
    assert_eq!(c_orig, c_copy);

    // Workspace scaffolded
    assert!(tmp.path().join("skillet.toml").is_file());
    assert!(tmp.path().join("skills/_fragments").is_dir());
}
