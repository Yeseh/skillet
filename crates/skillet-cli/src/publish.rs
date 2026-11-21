//! `skillet publish` — generate `plugin.json`/`marketplace.json` or `apm.yml` manifests.

use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use serde::Serialize;
use skillet::apm::PublishedPlugin;
use skillet::config::{PluginConfig, PublishConfig, SkilletConfig};
use std::path::Path;

// ── Options ────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PublishTarget {
    #[default]
    Direct,
    Apm,
}

pub struct PublishOptions {
    pub no_build: bool,
    pub format: OutputFormat,
    pub target: PublishTarget,
}

// ── Report ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PublishReport {
    pub plugins_published: Vec<String>,
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
        let build_opts = crate::build::BuildOptions {
            format: if opts.format == OutputFormat::Json {
                crate::build::OutputFormat::Quiet
            } else {
                crate::build::OutputFormat::Text
            },
            ..crate::build::BuildOptions::default()
        };
        crate::build::run(workspace_path, None, None, &build_opts, cfg)?;
    }

    // Stable ordering: sort by plugin name so emitted manifests are deterministic.
    let mut published_plugins: Vec<(&String, &PluginConfig)> =
        cfg.plugins.iter().filter(|(_, m)| m.publish).collect();
    published_plugins.sort_by_key(|(name, _)| name.as_str());

    if published_plugins.is_empty() {
        if opts.format != OutputFormat::Json {
            eprintln!("no plugins have publish = true");
        } else {
            let report = PublishReport {
                plugins_published: vec![],
                files_written: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        return Ok(());
    }

    match opts.target {
        PublishTarget::Direct => run_direct(workspace_path, opts, publish_cfg, &published_plugins),
        PublishTarget::Apm => run_apm(workspace_path, opts, publish_cfg, &published_plugins),
    }
}

fn run_apm(
    workspace_path: &Path,
    opts: &PublishOptions,
    publish_cfg: &PublishConfig,
    published_plugins: &[(&String, &PluginConfig)],
) -> Result<()> {
    let apm_plugins: Vec<PublishedPlugin<'_>> = published_plugins
        .iter()
        .map(|(name, cfg)| PublishedPlugin::from_config(name.as_str(), cfg))
        .collect();

    // Render and validate all manifests before writing any files to disk.
    let rendered_plugins: Vec<(&str, &str, String)> = published_plugins
        .iter()
        .zip(&apm_plugins)
        .map(|((plugin_name, plugin_cfg), published)| {
            let yaml = skillet::apm::render_plugin_manifest(published)?;
            Ok((plugin_name.as_str(), plugin_cfg.out_dir.as_str(), yaml))
        })
        .collect::<Result<Vec<_>>>()?;

    let root_apm_path = workspace_path.join("apm.yml");
    let existing_yaml = if root_apm_path.exists() {
        Some(
            std::fs::read_to_string(&root_apm_path)
                .with_context(|| format!("failed to read {}", root_apm_path.display()))?,
        )
    } else {
        None
    };

    let merged =
        skillet::apm::merge_root_manifest(existing_yaml.as_deref(), publish_cfg, &apm_plugins)?;

    let mut plugins_published = Vec::with_capacity(rendered_plugins.len());
    let mut files_written = Vec::with_capacity(rendered_plugins.len() + 1);

    for (plugin_name, out_dir_rel, yaml) in rendered_plugins {
        let out_dir = workspace_path.join(out_dir_rel);
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("failed to create out_dir for plugin '{}'", plugin_name))?;

        let plugin_apm_path = out_dir.join("apm.yml");
        std::fs::write(&plugin_apm_path, yaml)
            .with_context(|| format!("failed to write apm.yml for plugin '{}'", plugin_name))?;

        let rel_path = plugin_apm_path
            .strip_prefix(workspace_path)
            .unwrap_or(&plugin_apm_path)
            .to_string_lossy()
            .replace('\\', "/");

        if opts.format != OutputFormat::Json {
            println!("published plugin '{}' → {}", plugin_name, rel_path);
        }

        plugins_published.push(plugin_name.to_string());
        files_written.push(rel_path);
    }

    for warning in &merged.warnings {
        eprintln!("{} {}", "warning:".yellow(), warning);
    }

    std::fs::write(&root_apm_path, merged.yaml)
        .with_context(|| format!("failed to write {}", root_apm_path.display()))?;

    let root_rel = "apm.yml".to_string();
    if opts.format != OutputFormat::Json {
        println!("wrote apm manifest → {}", root_rel);
    }
    files_written.push(root_rel);

    if opts.format == OutputFormat::Json {
        let report = PublishReport {
            plugins_published,
            files_written,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn run_direct(
    workspace_path: &Path,
    opts: &PublishOptions,
    publish_cfg: &PublishConfig,
    published_plugins: &[(&String, &PluginConfig)],
) -> Result<()> {
    let mut plugins_published: Vec<String> = Vec::new();
    let mut files_written: Vec<String> = Vec::new();
    let mut plugin_entries: Vec<PluginEntry> = Vec::new();

    for (plugin_name, plugin_cfg) in published_plugins {
        let out_dir = workspace_path.join(&plugin_cfg.out_dir);
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("failed to create out_dir for plugin '{}'", plugin_name))?;

        let agent_files = published_agent_files(&out_dir)?;
        let extensions = (!agent_files.is_empty()).then(|| {
            serde_json::json!({
                skillet::agent::EXTENSION_NAMESPACE: {
                    "version": skillet::agent::EXTENSION_VERSION
                }
            })
        });

        let plugin_json = PluginJson {
            name: plugin_name.as_str(),
            version: &plugin_cfg.version,
            description: plugin_cfg.description.as_deref(),
            agents: None,
            extensions,
        };

        let plugin_json_path = out_dir.join("plugin.json");
        std::fs::write(
            &plugin_json_path,
            serde_json::to_string_pretty(&plugin_json)?,
        )
        .with_context(|| format!("failed to write plugin.json for plugin '{}'", plugin_name))?;

        let rel_path = plugin_json_path
            .strip_prefix(workspace_path)
            .unwrap_or(&plugin_json_path)
            .to_string_lossy()
            .replace('\\', "/");

        if opts.format != OutputFormat::Json {
            println!("published plugin '{}' → {}", plugin_name, rel_path);
        }

        files_written.push(rel_path);

        // Claude Code finds plugin agents through `.claude-plugin/plugin.json`,
        // which points at the canonical agent files (no copies).
        if !agent_files.is_empty() && publish_cfg.agents.iter().any(|a| a == "claude") {
            let claude_json = PluginJson {
                name: plugin_name.as_str(),
                version: &plugin_cfg.version,
                description: plugin_cfg.description.as_deref(),
                agents: Some(agent_files),
                extensions: None,
            };
            let claude_dir = out_dir.join(".claude-plugin");
            std::fs::create_dir_all(&claude_dir)
                .with_context(|| format!("failed to create {}", claude_dir.display()))?;
            let claude_path = claude_dir.join("plugin.json");
            std::fs::write(&claude_path, serde_json::to_string_pretty(&claude_json)?)
                .with_context(|| format!("failed to write {}", claude_path.display()))?;

            let rel = claude_path
                .strip_prefix(workspace_path)
                .unwrap_or(&claude_path)
                .to_string_lossy()
                .replace('\\', "/");
            if opts.format != OutputFormat::Json {
                println!("wrote claude plugin manifest → {}", rel);
            }
            files_written.push(rel);
        }

        plugins_published.push(plugin_name.to_string());

        plugin_entries.push(PluginEntry {
            name: plugin_name.to_string(),
            version: plugin_cfg.version.clone(),
            description: plugin_cfg.description.clone(),
            source: format!("./{}", plugin_cfg.out_dir.replace('\\', "/")),
        });
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
            plugins_published,
            files_written,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

// ── JSON output structures ─────────────────────────────────────────────────────

/// Lists the built agent definitions of a plugin as `./`-relative paths,
/// sorted, read from the agent extension directory in `out_dir`.
fn published_agent_files(out_dir: &Path) -> Result<Vec<String>> {
    let dir = out_dir
        .join(skillet::agent::EXTENSION_NAMESPACE)
        .join("agents");
    if !dir.is_dir() {
        return Ok(vec![]);
    }

    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
    {
        let path = entry?.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_file() && file_name.ends_with(skillet::agent::OUTPUT_SUFFIX) {
            files.push(format!(
                "./{}/agents/{}",
                skillet::agent::EXTENSION_NAMESPACE,
                file_name
            ));
        }
    }
    files.sort();
    Ok(files)
}

#[derive(Debug, Serialize)]
struct PluginJson<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    /// Claude Code only: agent files that replace the default `agents/` scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    agents: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extensions: Option<serde_json::Value>,
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
