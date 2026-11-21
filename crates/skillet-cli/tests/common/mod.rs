#![allow(dead_code)]

/// Path of the `skillet` binary under test. Cargo sets `CARGO_BIN_EXE_<name>`
/// for integration tests, so this works whatever the target directory layout.
pub fn skillet_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_skillet"))
}

pub fn run_skillet(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(skillet_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run skillet")
}
