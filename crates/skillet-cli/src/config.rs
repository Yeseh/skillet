use anyhow::Context;
use skillet::config::SkilletConfig;
use std::path::Path;

pub fn load(workspace: &Path) -> anyhow::Result<SkilletConfig> {
    let toml_path = workspace.join("skillet.toml");
    let raw = std::fs::read_to_string(&toml_path).with_context(|| {
        format!(
            "cannot read {}: workspace not initialized (run `skillet init` first)",
            toml_path.display()
        )
    })?;
    toml::from_str(&raw).context("failed to parse skillet.toml")
}
