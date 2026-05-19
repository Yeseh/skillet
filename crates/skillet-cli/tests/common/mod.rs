#![allow(dead_code)]

pub fn skillet_bin() -> std::path::PathBuf {
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

pub fn run_skillet(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(skillet_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run skillet")
}
