mod common;

use std::fs;
use std::path::Path;

#[test]
fn all_example_workspaces_build_successfully() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let examples_dir = manifest_dir.join("../../examples");

    assert!(
        examples_dir.is_dir(),
        "examples/ directory not found at {}",
        examples_dir.display()
    );

    let mut workspaces: Vec<std::path::PathBuf> = fs::read_dir(&examples_dir)
        .expect("failed to read examples/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("skillet.toml").exists())
        .collect();

    assert!(
        !workspaces.is_empty(),
        "no example workspaces found in examples/"
    );

    workspaces.sort();

    for workspace in &workspaces {
        let name = workspace.file_name().unwrap().to_string_lossy();

        // Copy to a temp dir so generated output doesn't pollute the source tree.
        let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
        copy_dir_all(workspace, tmp.path()).expect("failed to copy example workspace");

        let out = common::run_skillet(tmp.path(), &["build", "--offline"]);
        assert!(
            out.status.success(),
            "example '{}' failed to build:\nstdout: {}\nstderr: {}",
            name,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn url_verification_example_fails_with_strict() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let example = manifest_dir.join("../../examples/05-url-verification");

    assert!(
        example.is_dir(),
        "examples/05-url-verification not found at {}",
        example.display()
    );

    let tmp = tempfile::TempDir::new().expect("failed to create temp dir");
    copy_dir_all(&example, tmp.path()).expect("failed to copy example workspace");

    // Build without --offline so URL verification runs, and with --strict so
    // unreachable URLs are promoted to a hard error.
    let out = common::run_skillet(tmp.path(), &["build", "--strict"]);

    assert!(
        !out.status.success(),
        "expected build to fail due to unreachable URLs, but it succeeded"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("URL verification failed"),
        "expected 'URL verification failed' in stderr, got:\n{stderr}"
    );
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
