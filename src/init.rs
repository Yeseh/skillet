use anyhow::{bail, Context, Result};
use std::path::Path;
use walkdir::WalkDir;

pub fn run(workspace: &Path, adopt: bool) -> Result<()> {
    let config_path = workspace.join("skillet.toml");

    // Preflight: refuse to overwrite existing config
    if config_path.exists() {
        bail!(
            "skillet.toml already exists at {}; refusing to overwrite",
            config_path.display()
        );
    }

    let default_cfg = crate::config::render_default()?;

    // Parse the default config to know skills_dir/fragments_dir
    let skills_dir = workspace.join("skills");
    let fragments_dir = workspace.join("skills/_fragments");

    if adopt {
        adopt_skills(&skills_dir).context("failed to adopt SKILL.md files")?;
    }

    // Create directories
    std::fs::create_dir_all(&skills_dir).context("failed to create skills dir")?;
    std::fs::create_dir_all(&fragments_dir).context("failed to create fragments dir")?;

    // Write config
    std::fs::write(&config_path, &default_cfg).context("failed to write skillet.toml")?;

    Ok(())
}

fn adopt_skills(skills_dir: &Path) -> Result<()> {
    if !skills_dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(skills_dir)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
            continue;
        }
        let parent = match path.parent() {
            Some(p) => p,
            None => continue,
        };
        let dir_name = match parent.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let dest = parent.join(format!("{}.skill", dir_name));
        std::fs::copy(path, &dest)
            .with_context(|| format!("failed to copy {} to {}", path.display(), dest.display()))?;
    }
    Ok(())
}
