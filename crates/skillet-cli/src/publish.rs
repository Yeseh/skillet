//! `skillet publish` — generate `plugin.json` and `marketplace.json` manifests.

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use serde::Serialize;
use skillet::config::SkilletConfig;
use std::path::Path;

// ── Options ────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

pub struct PublishOptions {
    pub no_build: bool,
    pub format: OutputFormat,
}

// ── Report ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PublishReport {
    pub modules_published: Vec<String>,
    pub files_written: Vec<String>,
}

// ── Entry point ────────────────────────────────────────────────────────────────

pub fn run(workspace_path: &Path, opts: &PublishOptions, cfg: &SkilletConfig) -> Result<()> {
    let publish_cfg = cfg.workspace.publish.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "no [workspace.publish] config found — add a [workspace.publish] section to skillet.toml"
        )
    })?;

    if !opts.no_build {
        let build_opts = crate::build::BuildOptions::default();
        crate::build::run(workspace_path, None, None, &build_opts, cfg)?;
    }

    let mut modules_published: Vec<String> = Vec::new();
    let mut files_written: Vec<String> = Vec::new();
    let mut plugin_entries: Vec<PluginEntry> = Vec::new();

    // Stable ordering: sort by module name so marketplace.json is deterministic.
    let mut published_modules: Vec<(&String, &skillet::config::ModuleConfig)> =
        cfg.modules.iter().filter(|(_, m)| m.publish).collect();
    published_modules.sort_by_key(|(name, _)| name.as_str());

    for (module_name, module_cfg) in &published_modules {
        let out_dir = workspace_path.join(&module_cfg.out_dir);
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("failed to create out_dir for module '{}'", module_name))?;

        let plugin_json = PluginJson {
            name: module_name.as_str(),
            version: &module_cfg.version,
            description: module_cfg.description.as_deref(),
        };

        let plugin_json_path = out_dir.join("plugin.json");
        std::fs::write(
            &plugin_json_path,
            serde_json::to_string_pretty(&plugin_json)?,
        )
        .with_context(|| format!("failed to write plugin.json for module '{}'", module_name))?;

        let rel_path = plugin_json_path
            .strip_prefix(workspace_path)
            .unwrap_or(&plugin_json_path)
            .to_string_lossy()
            .replace('\\', "/");

        if opts.format != OutputFormat::Json {
            println!("published module '{}' → {}", module_name, rel_path);
        }

        files_written.push(rel_path);
        modules_published.push(module_name.to_string());

        plugin_entries.push(PluginEntry {
            name: module_name.to_string(),
            version: module_cfg.version.clone(),
            description: module_cfg.description.clone(),
            source: format!("./{}", module_cfg.out_dir.replace('\\', "/")),
        });
    }

    if modules_published.is_empty() {
        if opts.format != OutputFormat::Json {
            eprintln!("no modules have publish = true");
        } else {
            let report = PublishReport {
                modules_published: vec![],
                files_written: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        return Ok(());
    }

    // Write marketplace.json for each configured agent.
    for agent in &publish_cfg.agents {
        let agent_dir = match agent.as_str() {
            "claude" => ".claude-plugin",
            "github-copilot" => ".github/plugin",
            other => {
                eprintln!(
                    "{} unknown agent '{}' — skipping (supported: claude, github-copilot)",
                    "warning:".yellow(),
                    other
                );
                continue;
            }
        };

        let dir_path = workspace_path.join(agent_dir);
        std::fs::create_dir_all(&dir_path)
            .with_context(|| format!("failed to create directory '{}'", agent_dir))?;

        let marketplace = MarketplaceJson {
            name: &publish_cfg.marketplace_name,
            owner: Owner {
                name: &publish_cfg.owner_name,
                email: publish_cfg.owner_email.as_deref(),
            },
            plugins: &plugin_entries,
        };

        let marketplace_path = dir_path.join("marketplace.json");
        std::fs::write(
            &marketplace_path,
            serde_json::to_string_pretty(&marketplace)?,
        )
        .with_context(|| format!("failed to write marketplace.json for agent '{}'", agent))?;

        let rel_path = format!("{}/marketplace.json", agent_dir);
        if opts.format != OutputFormat::Json {
            println!("wrote marketplace → {}", rel_path);
        }
        files_written.push(rel_path);
    }

    if opts.format == OutputFormat::Json {
        let report = PublishReport {
            modules_published,
            files_written,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

// ── JSON output structures ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct PluginJson<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct MarketplaceJson<'a> {
    name: &'a str,
    owner: Owner<'a>,
    plugins: &'a [PluginEntry],
}

#[derive(Debug, Serialize)]
struct Owner<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct PluginEntry {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    source: String,
}
