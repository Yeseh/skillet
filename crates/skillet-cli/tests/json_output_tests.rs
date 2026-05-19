
mod common;

use tempfile::TempDir;

fn setup_workspace_with_skill(name: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", name]);
    common::run_skillet(tmp.path(), &["build"]);
    tmp
}

// ── init --format json ────────────────────────────────────────────────────────

#[test]
fn init_json_produces_clean_json_on_stdout() {
    let tmp = TempDir::new().unwrap();
    let out = common::run_skillet(tmp.path(), &["init", "--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(v.get("config_path").is_some(), "should have config_path");
    assert!(v.get("created_dirs").is_some(), "should have created_dirs");
}

#[test]
fn init_json_exit_code_matches_default() {
    let tmp_h = TempDir::new().unwrap();
    let out_h = common::run_skillet(tmp_h.path(), &["init"]);
    let tmp_j = TempDir::new().unwrap();
    let out_j = common::run_skillet(tmp_j.path(), &["init", "--format", "json"]);
    assert_eq!(
        out_h.status.code(),
        out_j.status.code(),
        "exit codes should match"
    );
}

// ── new --format json ─────────────────────────────────────────────────────────

#[test]
fn new_json_produces_clean_json_on_stdout() {
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    let out = common::run_skillet(tmp.path(), &["new", "my-skill", "--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(v.get("created").is_some(), "should have created field");
}

#[test]
fn new_json_exit_code_matches_default() {
    let tmp_h = TempDir::new().unwrap();
    common::run_skillet(tmp_h.path(), &["init"]);
    let out_h = common::run_skillet(tmp_h.path(), &["new", "my-skill"]);
    let tmp_j = TempDir::new().unwrap();
    common::run_skillet(tmp_j.path(), &["init"]);
    let out_j = common::run_skillet(tmp_j.path(), &["new", "my-skill", "--format", "json"]);
    assert_eq!(out_h.status.code(), out_j.status.code());
}

// ── build --format json ───────────────────────────────────────────────────────

#[test]
fn build_json_produces_clean_json_on_stdout() {
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    common::run_skillet(tmp.path(), &["new", "my-skill"]);
    let out = common::run_skillet(tmp.path(), &["build", "--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(v.get("skills_built").is_some());
    assert!(v.get("warnings").is_some());
    assert!(v.get("lockfile_path").is_some());
    let skills = v["skills_built"].as_array().unwrap();
    assert!(
        skills.iter().any(|s| s == "my-skill"),
        "skills_built should include my-skill"
    );
}

#[test]
fn build_json_exit_code_matches_default() {
    let tmp_h = TempDir::new().unwrap();
    common::run_skillet(tmp_h.path(), &["init"]);
    common::run_skillet(tmp_h.path(), &["new", "my-skill"]);
    let out_h = common::run_skillet(tmp_h.path(), &["build"]);
    let tmp_j = TempDir::new().unwrap();
    common::run_skillet(tmp_j.path(), &["init"]);
    common::run_skillet(tmp_j.path(), &["new", "my-skill"]);
    let out_j = common::run_skillet(tmp_j.path(), &["build", "--format", "json"]);
    assert_eq!(out_h.status.code(), out_j.status.code());
}

// ── budget --format json ──────────────────────────────────────────────────────

#[test]
fn budget_json_includes_skills_and_totals() {
    let tmp = setup_workspace_with_skill("my-skill");
    let out = common::run_skillet(tmp.path(), &["budget", "--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(v.get("skills").is_some(), "should have skills array");
    assert!(v.get("totals").is_some(), "should have totals");
    let totals = &v["totals"];
    assert!(totals.get("discovery").is_some());
    assert!(totals.get("activation").is_some());
    assert!(totals.get("transitive").is_some());
}

#[test]
fn budget_json_stdout_is_clean_json() {
    let tmp = setup_workspace_with_skill("my-skill");
    let out = common::run_skillet(tmp.path(), &["budget", "--format", "json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should parse cleanly — no extra lines
    serde_json::from_str::<serde_json::Value>(&stdout).expect("stdout should be parseable JSON");
}

// ── check --format json ───────────────────────────────────────────────────────

#[test]
fn check_json_produces_clean_json_with_diffs() {
    let tmp = setup_workspace_with_skill("my-skill");
    let out = common::run_skillet(tmp.path(), &["check", "--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(v.get("fresh").is_some());
    assert!(v.get("skills").is_some());
    let skill = &v["skills"][0];
    assert!(
        skill.get("diffs").is_some(),
        "skill should have diffs field"
    );
}

#[test]
fn check_json_stale_skill_has_diff_entries() {
    let tmp = setup_workspace_with_skill("my-skill");
    // Modify source to make it stale
    let src = tmp.path().join("src/skills/my-skill/my-skill.pan");
    let original = std::fs::read_to_string(&src).unwrap();
    std::fs::write(&src, format!("{}\n## Extra\n", original)).unwrap();

    let out = common::run_skillet(tmp.path(), &["check", "--format", "json"]);
    assert!(!out.status.success(), "should exit non-zero when stale");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    let skill = &v["skills"][0];
    assert_eq!(skill["fresh"], false);
    let diffs = skill["diffs"].as_array().unwrap();
    assert!(!diffs.is_empty(), "stale skill should have diff entries");
    assert!(diffs.iter().any(|d| d["kind"] == "source_changed"));
}

#[test]
fn check_json_exit_code_matches_default() {
    // Both fresh
    let tmp_h = setup_workspace_with_skill("my-skill");
    let out_h = common::run_skillet(tmp_h.path(), &["check"]);
    let tmp_j = setup_workspace_with_skill("my-skill");
    let out_j = common::run_skillet(tmp_j.path(), &["check", "--format", "json"]);
    assert_eq!(out_h.status.code(), out_j.status.code());
}

// ── lint --format json ────────────────────────────────────────────────────────

#[test]
fn lint_json_produces_clean_json() {
    let tmp = setup_workspace_with_skill("my-skill");
    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    assert!(v.is_array(), "lint JSON should be an array");
}

#[test]
fn lint_json_diagnostic_has_required_fields() {
    let tmp = TempDir::new().unwrap();
    common::run_skillet(tmp.path(), &["init"]);
    // Create a skill with a bad frontmatter name to trigger a diagnostic
    let skill_dir = tmp.path().join("src/skills/my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("my-skill.pan"),
        "---\nname: wrong-name\ndescription: x\n---\n\n# body\n",
    )
    .unwrap();

    let out = common::run_skillet(tmp.path(), &["lint", "--format", "json"]);
    assert!(!out.status.success(), "should fail with lint errors");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let diags: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));
    let arr = diags.as_array().unwrap();
    assert!(!arr.is_empty(), "should have diagnostics");
    let d = &arr[0];
    assert!(d.get("rule").is_some(), "diagnostic needs rule");
    assert!(d.get("severity").is_some(), "diagnostic needs severity");
    assert!(d.get("message").is_some(), "diagnostic needs message");
    assert!(d.get("skill").is_some(), "diagnostic needs skill");
}

#[test]
fn lint_json_exit_code_matches_default() {
    // Clean workspace
    let tmp_h = setup_workspace_with_skill("my-skill");
    let out_h = common::run_skillet(tmp_h.path(), &["lint"]);
    let tmp_j = setup_workspace_with_skill("my-skill");
    let out_j = common::run_skillet(tmp_j.path(), &["lint", "--format", "json"]);
    assert_eq!(out_h.status.code(), out_j.status.code());
}

// ── stdout / stderr separation ────────────────────────────────────────────────

#[test]
fn json_mode_stdout_contains_only_json() {
    // For each command, stdout in JSON mode must be parseable JSON (no extra lines).
    let tmp = setup_workspace_with_skill("my-skill");

    for args in &[
        vec!["build", "--format", "json"],
        vec!["budget", "--format", "json"],
        vec!["check", "--format", "json"],
        vec!["lint", "--format", "json"],
    ] {
        let out = common::run_skillet(tmp.path(), args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str::<serde_json::Value>(&stdout)
            .unwrap_or_else(|e| panic!("non-JSON stdout for {:?}: {e}\nstdout: {stdout}", args));
    }
}
