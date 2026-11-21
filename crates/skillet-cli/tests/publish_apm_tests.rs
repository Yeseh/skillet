mod common;

use std::fs;
use tempfile::TempDir;

fn setup_apm_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();

    fs::write(
        tmp.path().join("skillet.toml"),
        r#"[workspace.publish]
agents = ["claude", "github-copilot"]
marketplace_name = "my-marketplace"
owner_name = "Contoso"
owner_email = "maintainers@contoso.example"

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"

[plugin.core]
src_dir = "src/skills"
out_dir = "skills"
version = "1.0.0"
description = "Core workflow skills"
publish = true
dependencies = [
  "microsoft/apm-sample-package#v1.0.0",
  { git = "https://gitlab.com/acme/repo.git", path = "instructions/security", ref = "v2.0", alias = "acme-sec" },
  { name = "code-review", marketplace = "contoso", version = "^1.0.0" },
]

[plugin.internal]
src_dir = "src/internal"
out_dir = "internal"
version = "0.1.0"
publish = false
"#,
    )
    .unwrap();

    let skill_dir = tmp.path().join("src/skills/hello");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("hello.pan"),
        "---\nname: hello\ndescription: Say hello\n---\n\n# Hello\n\nGreet the user.\n",
    )
    .unwrap();

    let internal_dir = tmp.path().join("src/internal/secret");
    fs::create_dir_all(&internal_dir).unwrap();
    fs::write(
        internal_dir.join("secret.pan"),
        "---\nname: secret\ndescription: Internal skill\n---\n\n# Secret\n",
    )
    .unwrap();

    tmp
}

#[test]
fn publish_target_apm_builds_and_writes_plugin_and_root_manifests() {
    let tmp = setup_apm_workspace();

    let out = common::run_skillet(tmp.path(), &["publish", "--target", "apm"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Pre-build compiled skills.
    assert!(tmp.path().join("skills/hello/SKILL.md").is_file());

    // Published plugin gets apm.yml; unpublished plugin does not.
    let plugin_apm_path = tmp.path().join("skills/apm.yml");
    assert!(plugin_apm_path.is_file());
    assert!(!tmp.path().join("internal/apm.yml").exists());
    assert!(!tmp.path().join("skills/plugin.json").exists());
    assert!(!tmp.path().join(".claude-plugin/marketplace.json").exists());

    let plugin_apm = fs::read_to_string(&plugin_apm_path).unwrap();
    assert!(plugin_apm.contains("name: core"));
    assert!(plugin_apm.contains("version: 1.0.0"));
    assert!(plugin_apm.contains("description: Core workflow skills"));
    assert!(plugin_apm.contains("microsoft/apm-sample-package#v1.0.0"));
    assert!(plugin_apm.contains("git: https://gitlab.com/acme/repo.git"));
    assert!(plugin_apm.contains("marketplace: contoso"));

    // Root apm.yml scaffolded with marketplace block.
    let root_apm_path = tmp.path().join("apm.yml");
    assert!(root_apm_path.is_file());
    let root_apm = fs::read_to_string(&root_apm_path).unwrap();
    assert!(root_apm.contains("name: my-marketplace"));
    assert!(root_apm.contains("version: 0.1.0"));
    assert!(root_apm.contains("name: Contoso"));
    assert!(root_apm.contains("email: maintainers@contoso.example"));
    assert!(root_apm.contains("claude: {}"));
    assert!(root_apm.contains("copilot: {}"));
    assert!(root_apm.contains("source: ./skills"));
    assert!(!root_apm.contains("internal"));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("published plugin 'core' → skills/apm.yml"));
    assert!(stdout.contains("wrote apm manifest → apm.yml"));
}

#[test]
fn publish_target_apm_preserves_existing_root_manifest_and_is_idempotent() {
    let tmp = setup_apm_workspace();

    let existing_root = r#"name: hand-authored-root
version: 2.0.0
scripts:
  check: skillet check
dependencies:
  apm:
    - org/shared-rules#v1.0.0
x-meta:
  owner_team: devex
marketplace:
  sourceBase: https://github.com/contoso/repo
  owner:
    name: OldName
    url: https://contoso.example
"#;
    fs::write(tmp.path().join("apm.yml"), existing_root).unwrap();

    let first = common::run_skillet(tmp.path(), &["publish", "--target", "apm"]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let after_first = fs::read_to_string(tmp.path().join("apm.yml")).unwrap();

    assert!(after_first.contains("name: hand-authored-root"));
    assert!(after_first.contains("version: 2.0.0"));
    assert!(after_first.contains("check: skillet check"));
    assert!(after_first.contains("org/shared-rules#v1.0.0"));
    assert!(after_first.contains("owner_team: devex"));
    assert!(after_first.contains("sourceBase: https://github.com/contoso/repo"));
    assert!(after_first.contains("url: https://contoso.example"));
    assert!(after_first.contains("name: Contoso"));
    assert!(after_first.contains("source: ./skills"));

    // Second run is byte-for-byte idempotent.
    let second = common::run_skillet(tmp.path(), &["publish", "--target", "apm"]);
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let after_second = fs::read_to_string(tmp.path().join("apm.yml")).unwrap();
    assert_eq!(after_first, after_second);
}

#[test]
fn publish_target_apm_no_build_skips_compilation_and_json_is_clean() {
    let tmp = setup_apm_workspace();

    let out = common::run_skillet(
        tmp.path(),
        &[
            "publish",
            "--target",
            "apm",
            "--no-build",
            "--format",
            "json",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Compilation was skipped, so SKILL.md was not generated.
    assert!(!tmp.path().join("skills/hello/SKILL.md").exists());
    // Manifests were still written.
    assert!(tmp.path().join("skills/apm.yml").is_file());
    assert!(tmp.path().join("apm.yml").is_file());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));

    assert_eq!(report["plugins_published"], serde_json::json!(["core"]));
    assert_eq!(
        report["files_written"],
        serde_json::json!(["skills/apm.yml", "apm.yml"])
    );
}

#[test]
fn publish_target_apm_json_with_build_emits_clean_json() {
    let tmp = setup_apm_workspace();

    let out = common::run_skillet(
        tmp.path(),
        &["publish", "--target", "apm", "--format", "json"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(tmp.path().join("skills/hello/SKILL.md").is_file());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout: {stdout}"));

    assert_eq!(report["plugins_published"], serde_json::json!(["core"]));
    assert_eq!(
        report["files_written"],
        serde_json::json!(["skills/apm.yml", "apm.yml"])
    );
}

#[test]
fn publish_default_and_target_direct_do_not_create_apm_yml() {
    let tmp_default = setup_apm_workspace();
    let out = common::run_skillet(tmp_default.path(), &["publish"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(tmp_default.path().join("skills/plugin.json").is_file());
    assert!(tmp_default
        .path()
        .join(".claude-plugin/marketplace.json")
        .is_file());
    assert!(tmp_default
        .path()
        .join(".github/plugin/marketplace.json")
        .is_file());
    assert!(!tmp_default.path().join("apm.yml").exists());
    assert!(!tmp_default.path().join("skills/apm.yml").exists());

    let tmp_direct = setup_apm_workspace();
    let out = common::run_skillet(tmp_direct.path(), &["publish", "--target", "direct"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(tmp_direct.path().join("skills/plugin.json").is_file());
    assert!(tmp_direct
        .path()
        .join(".claude-plugin/marketplace.json")
        .is_file());
    assert!(!tmp_direct.path().join("apm.yml").exists());
    assert!(!tmp_direct.path().join("skills/apm.yml").exists());
}

#[test]
fn publish_target_apm_with_no_published_plugins_does_not_touch_apm_yml() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("skillet.toml"),
        r#"[workspace.publish]
agents = ["claude"]
marketplace_name = "my-marketplace"
owner_name = "Contoso"

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500

[build]
tokenizer = "cl100k_base"

[plugin.internal]
src_dir = "src/internal"
out_dir = "internal"
version = "0.1.0"
publish = false
"#,
    )
    .unwrap();

    let out = common::run_skillet(tmp.path(), &["publish", "--target", "apm", "--no-build"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no plugins have publish = true"));
    assert!(!tmp.path().join("apm.yml").exists());

    let out_json = common::run_skillet(
        tmp.path(),
        &[
            "publish",
            "--target",
            "apm",
            "--no-build",
            "--format",
            "json",
        ],
    );
    assert!(out_json.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out_json.stdout).expect("valid empty JSON report");
    assert_eq!(report["plugins_published"], serde_json::json!([]));
    assert_eq!(report["files_written"], serde_json::json!([]));
    assert!(!tmp.path().join("apm.yml").exists());
}
