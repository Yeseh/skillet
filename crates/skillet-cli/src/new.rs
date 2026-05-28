//! `skillet new` — scaffold a new skill source inside an initialized workspace.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;

/// Report produced by `new` in JSON mode.
#[derive(Debug, Serialize)]
pub struct NewReport {
    pub created: String,
}

pub fn run(skills_src_dir: &Path, name: &str, json: bool) -> Result<()> {
    let skill_dir = skills_src_dir.join(name);
    let skill_file = skill_dir.join(format!("{name}.pan"));

    if skill_dir.exists() {
        bail!("skill '{name}' already exists at {}", skill_dir.display());
    }

    std::fs::create_dir_all(&skill_dir)
        .with_context(|| format!("failed to create directory {}", skill_dir.display()))?;

    std::fs::write(
        &skill_file,
        format!("---\nname: {name}\ndescription: \"TODO: describe this skill\"\n---\n\n# {name}\n"),
    )
    .with_context(|| format!("failed to write {}", skill_file.display()))?;

    if json {
        let report = NewReport {
            created: skill_file.to_string_lossy().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("created {}", skill_file.display());
    }
    Ok(())
}
