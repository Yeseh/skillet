//! Pure APM (`apm.yml`) manifest rendering, merging, and validation.
//!
//! Per ADR-0004, this module performs no filesystem or I/O operations.

use crate::config::{
    ApmDependency, GitApmDependency, LocalApmDependency, MarketplaceApmDependency, PluginConfig,
    PublishConfig, RegistryApmDependency,
};
use serde::Serialize;
use serde_yaml_ng::{Mapping, Value};
use std::fmt;

/// Metadata for a single published plugin passed to APM manifest generators.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PublishedPlugin<'a> {
    /// Plugin name (from `[plugin.<name>]`).
    pub name: &'a str,
    /// Plugin output directory relative to the workspace root.
    pub out_dir: &'a str,
    /// Plugin semantic version string.
    pub version: &'a str,
    /// Optional human-readable description.
    pub description: Option<&'a str>,
    /// External APM dependencies declared for this plugin.
    pub dependencies: &'a [ApmDependency],
}

impl<'a> PublishedPlugin<'a> {
    /// Creates a new [`PublishedPlugin`] from explicit metadata fields.
    pub fn new(
        name: &'a str,
        out_dir: &'a str,
        version: &'a str,
        description: Option<&'a str>,
        dependencies: &'a [ApmDependency],
    ) -> Self {
        Self {
            name,
            out_dir,
            version,
            description,
            dependencies,
        }
    }

    /// Creates a [`PublishedPlugin`] from a plugin's name and [`PluginConfig`].
    pub fn from_config(name: &'a str, cfg: &'a PluginConfig) -> Self {
        Self {
            name,
            out_dir: &cfg.out_dir,
            version: &cfg.version,
            description: cfg.description.as_deref(),
            dependencies: &cfg.dependencies,
        }
    }
}

/// Output of merging or scaffolding the root `apm.yml`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedRootManifest {
    /// Serialized YAML 1.2 content for root `apm.yml`.
    pub yaml: String,
    /// Non-fatal warnings (e.g. unrecognized agent names in `[workspace.publish].agents`).
    pub warnings: Vec<String>,
}

/// Typed errors produced while validating or rendering APM manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApmError {
    /// Existing root `apm.yml` could not be parsed as valid YAML.
    InvalidExistingYaml(String),
    /// Existing root `apm.yml` top-level value is not a YAML mapping.
    RootNotMapping,
    /// Existing `marketplace` entry in root `apm.yml` is not a YAML mapping.
    MarketplaceNotMapping,
    /// A plugin's `out_dir` cannot be normalized into a valid APM package source.
    InvalidPackageSource {
        /// Plugin name whose `out_dir` failed validation.
        plugin: String,
        /// Raw `out_dir` value.
        source: String,
        /// Human-readable reason the source was rejected.
        reason: String,
    },
    /// A plugin's `dependencies` entry failed validation.
    InvalidDependency {
        /// Plugin name owning the invalid dependency.
        plugin: String,
        /// Human-readable reason the dependency was rejected.
        reason: String,
    },
    /// Serializing the YAML document failed.
    Serialize(String),
}

impl fmt::Display for ApmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExistingYaml(msg) => {
                write!(f, "failed to parse existing apm.yml: {msg}")
            }
            Self::RootNotMapping => {
                write!(f, "existing apm.yml must have a YAML mapping at the root")
            }
            Self::MarketplaceNotMapping => {
                write!(
                    f,
                    "existing apm.yml field 'marketplace' must be a YAML mapping"
                )
            }
            Self::InvalidPackageSource {
                plugin,
                source,
                reason,
            } => write!(
                f,
                "invalid package source '{source}' for plugin '{plugin}': {reason}"
            ),
            Self::InvalidDependency { plugin, reason } => {
                write!(f, "invalid dependency for plugin '{plugin}': {reason}")
            }
            Self::Serialize(msg) => write!(f, "failed to serialize apm.yml: {msg}"),
        }
    }
}

impl std::error::Error for ApmError {}

#[derive(Serialize)]
struct PluginApmManifest<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<PluginApmDependencies<'a>>,
}

#[derive(Serialize)]
struct PluginApmDependencies<'a> {
    apm: &'a [ApmDependency],
}

/// Renders a standalone per-plugin `apm.yml` (pure).
pub fn render_plugin_manifest(plugin: &PublishedPlugin<'_>) -> Result<String, ApmError> {
    for dep in plugin.dependencies {
        validate_dependency(plugin.name, dep)?;
    }

    let dependencies = (!plugin.dependencies.is_empty()).then_some(PluginApmDependencies {
        apm: plugin.dependencies,
    });

    let manifest = PluginApmManifest {
        name: plugin.name,
        version: plugin.version,
        description: plugin.description,
        dependencies,
    };

    serde_yaml_ng::to_string(&manifest).map_err(|e| ApmError::Serialize(e.to_string()))
}

/// Scaffolds or merges the `marketplace:` block into the workspace root `apm.yml` (pure).
pub fn merge_root_manifest(
    existing_yaml: Option<&str>,
    publish_cfg: &PublishConfig,
    plugins: &[PublishedPlugin<'_>],
) -> Result<MergedRootManifest, ApmError> {
    let mut root = match existing_yaml {
        Some(raw) => {
            let value: Value = serde_yaml_ng::from_str(raw)
                .map_err(|e| ApmError::InvalidExistingYaml(e.to_string()))?;
            match value {
                Value::Mapping(map) => map,
                _ => return Err(ApmError::RootNotMapping),
            }
        }
        None => {
            let mut map = Mapping::new();
            map.insert(
                yaml_str("name"),
                Value::String(publish_cfg.marketplace_name.clone()),
            );
            map.insert(yaml_str("version"), Value::String("0.1.0".to_string()));
            map
        }
    };

    let marketplace_key = yaml_str("marketplace");
    let mut marketplace = match root.get(&marketplace_key) {
        Some(Value::Mapping(map)) => map.clone(),
        Some(_) => return Err(ApmError::MarketplaceNotMapping),
        None => Mapping::new(),
    };

    update_owner(&mut marketplace, publish_cfg);
    let warnings = update_outputs(&mut marketplace, publish_cfg);
    update_packages(&mut marketplace, plugins)?;

    root.insert(marketplace_key, Value::Mapping(marketplace));

    let yaml = serde_yaml_ng::to_string(&Value::Mapping(root))
        .map_err(|e| ApmError::Serialize(e.to_string()))?;

    Ok(MergedRootManifest { yaml, warnings })
}

fn yaml_str(s: &str) -> Value {
    Value::String(s.to_string())
}

fn update_owner(marketplace: &mut Mapping, publish_cfg: &PublishConfig) {
    let owner_key = yaml_str("owner");
    let mut owner = match marketplace.get(&owner_key) {
        Some(Value::Mapping(existing)) => existing.clone(),
        _ => Mapping::new(),
    };

    owner.insert(
        yaml_str("name"),
        Value::String(publish_cfg.owner_name.clone()),
    );
    let email_key = yaml_str("email");
    match &publish_cfg.owner_email {
        Some(email) => {
            owner.insert(email_key, Value::String(email.clone()));
        }
        None => {
            owner.remove(&email_key);
        }
    }

    marketplace.insert(owner_key, Value::Mapping(owner));
}

fn update_outputs(marketplace: &mut Mapping, publish_cfg: &PublishConfig) -> Vec<String> {
    let mut outputs = Mapping::new();
    let mut warnings = Vec::new();

    for agent in &publish_cfg.agents {
        match agent.as_str() {
            "claude" => {
                outputs.insert(yaml_str("claude"), Value::Mapping(Mapping::new()));
            }
            "github-copilot" => {
                outputs.insert(yaml_str("copilot"), Value::Mapping(Mapping::new()));
            }
            other => {
                warnings.push(format!(
                    "unknown agent '{other}' — skipping (supported: claude, github-copilot)"
                ));
            }
        }
    }

    marketplace.insert(yaml_str("outputs"), Value::Mapping(outputs));
    warnings
}

fn update_packages(
    marketplace: &mut Mapping,
    plugins: &[PublishedPlugin<'_>],
) -> Result<(), ApmError> {
    let mut sorted: Vec<&PublishedPlugin<'_>> = plugins.iter().collect();
    sorted.sort_by_key(|p| p.name);

    let mut packages = Vec::with_capacity(sorted.len());
    for plugin in sorted {
        let source = normalize_package_source(plugin.name, plugin.out_dir)?;
        let mut entry = Mapping::new();
        entry.insert(yaml_str("name"), yaml_str(plugin.name));
        entry.insert(yaml_str("source"), Value::String(source));
        entry.insert(yaml_str("version"), yaml_str(plugin.version));
        if let Some(description) = plugin.description {
            entry.insert(yaml_str("description"), yaml_str(description));
        }
        packages.push(Value::Mapping(entry));
    }

    marketplace.insert(yaml_str("packages"), Value::Sequence(packages));
    Ok(())
}

fn normalize_package_source(plugin: &str, out_dir: &str) -> Result<String, ApmError> {
    let trimmed = out_dir.trim();
    if trimmed.is_empty() {
        return Err(ApmError::InvalidPackageSource {
            plugin: plugin.to_string(),
            source: out_dir.to_string(),
            reason: "out_dir must not be empty".to_string(),
        });
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized.starts_with('/') || has_windows_drive_prefix(&normalized) {
        return Err(ApmError::InvalidPackageSource {
            plugin: plugin.to_string(),
            source: out_dir.to_string(),
            reason: "out_dir must be a relative path".to_string(),
        });
    }

    let mut segments = Vec::new();
    for segment in normalized.split('/') {
        if segment == ".." {
            return Err(ApmError::InvalidPackageSource {
                plugin: plugin.to_string(),
                source: out_dir.to_string(),
                reason: "out_dir must not contain '..' traversal segments".to_string(),
            });
        }
        if segment.is_empty() || segment == "." {
            continue;
        }
        segments.push(segment);
    }

    if segments.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(format!("./{}", segments.join("/")))
    }
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validate_dependency(plugin: &str, dep: &ApmDependency) -> Result<(), ApmError> {
    match dep {
        ApmDependency::Shorthand(s) => require_non_empty(plugin, "shorthand", s),
        ApmDependency::Git(g) => validate_git_dep(plugin, g),
        ApmDependency::Local(l) => validate_local_dep(plugin, l),
        ApmDependency::Marketplace(m) => validate_marketplace_dep(plugin, m),
        ApmDependency::Registry(r) => validate_registry_dep(plugin, r),
    }
}

fn validate_git_dep(plugin: &str, dep: &GitApmDependency) -> Result<(), ApmError> {
    require_non_empty(plugin, "git", &dep.git)?;
    validate_opt_non_empty(plugin, "path", dep.path.as_deref())?;
    validate_opt_non_empty(plugin, "ref", dep.git_ref.as_deref())?;
    validate_opt_non_empty(plugin, "alias", dep.alias.as_deref())?;
    validate_opt_non_empty(plugin, "type", dep.repo_type.as_deref())?;
    validate_string_list(plugin, "skills", dep.skills.as_deref(), false)?;
    validate_string_list(plugin, "targets", dep.targets.as_deref(), true)?;
    Ok(())
}

fn validate_local_dep(plugin: &str, dep: &LocalApmDependency) -> Result<(), ApmError> {
    require_non_empty(plugin, "path", &dep.path)?;
    validate_opt_non_empty(plugin, "alias", dep.alias.as_deref())?;
    validate_string_list(plugin, "skills", dep.skills.as_deref(), false)?;
    validate_string_list(plugin, "targets", dep.targets.as_deref(), true)?;
    Ok(())
}

fn validate_marketplace_dep(plugin: &str, dep: &MarketplaceApmDependency) -> Result<(), ApmError> {
    require_non_empty(plugin, "name", &dep.name)?;
    require_non_empty(plugin, "marketplace", &dep.marketplace)?;
    validate_opt_non_empty(plugin, "version", dep.version.as_deref())?;
    Ok(())
}

fn validate_registry_dep(plugin: &str, dep: &RegistryApmDependency) -> Result<(), ApmError> {
    require_non_empty(plugin, "id", &dep.id)?;
    validate_opt_non_empty(plugin, "registry", dep.registry.as_deref())?;
    validate_opt_non_empty(plugin, "version", dep.version.as_deref())?;
    Ok(())
}

fn require_non_empty(plugin: &str, field: &str, value: &str) -> Result<(), ApmError> {
    if value.trim().is_empty() {
        return Err(ApmError::InvalidDependency {
            plugin: plugin.to_string(),
            reason: format!("'{field}' must not be empty"),
        });
    }
    Ok(())
}

fn validate_opt_non_empty(plugin: &str, field: &str, value: Option<&str>) -> Result<(), ApmError> {
    if let Some(v) = value {
        require_non_empty(plugin, field, v)?;
    }
    Ok(())
}

fn validate_string_list(
    plugin: &str,
    field: &str,
    values: Option<&[String]>,
    reject_empty_list: bool,
) -> Result<(), ApmError> {
    let Some(items) = values else {
        return Ok(());
    };
    if reject_empty_list && items.is_empty() {
        return Err(ApmError::InvalidDependency {
            plugin: plugin.to_string(),
            reason: format!("'{field}' must not be an empty list"),
        });
    }
    for item in items {
        if item.trim().is_empty() {
            return Err(ApmError::InvalidDependency {
                plugin: plugin.to_string(),
                reason: format!("'{field}' must not contain empty strings"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_publish_cfg() -> PublishConfig {
        PublishConfig {
            agents: vec!["claude".to_string(), "github-copilot".to_string()],
            marketplace_name: "my-marketplace".to_string(),
            owner_name: "Contoso".to_string(),
            owner_email: Some("maintainers@contoso.example".to_string()),
        }
    }

    #[test]
    fn render_plugin_manifest_basic_without_dependencies() {
        let plugin =
            PublishedPlugin::new("core", "skills", "1.0.0", Some("Core workflow skills"), &[]);

        let yaml = render_plugin_manifest(&plugin).unwrap();
        let parsed: Value = serde_yaml_ng::from_str(&yaml).unwrap();

        assert_eq!(parsed["name"].as_str(), Some("core"));
        assert_eq!(parsed["version"].as_str(), Some("1.0.0"));
        assert_eq!(parsed["description"].as_str(), Some("Core workflow skills"));
        assert!(parsed.get("dependencies").is_none());
    }

    #[test]
    fn render_plugin_manifest_omits_absent_description() {
        let plugin = PublishedPlugin::new("minimal", "skills", "0.1.0", None, &[]);
        let yaml = render_plugin_manifest(&plugin).unwrap();
        let parsed: Value = serde_yaml_ng::from_str(&yaml).unwrap();

        assert_eq!(parsed["name"].as_str(), Some("minimal"));
        assert!(parsed.get("description").is_none());
        assert!(parsed.get("dependencies").is_none());
    }

    #[test]
    fn render_plugin_manifest_emits_all_dependency_forms() {
        let deps = vec![
            ApmDependency::Shorthand("microsoft/apm-sample-package#v1.0.0".to_string()),
            ApmDependency::Git(GitApmDependency {
                git: "https://gitlab.com/acme/repo.git".to_string(),
                path: Some("instructions/security".to_string()),
                git_ref: Some("v2.0".to_string()),
                alias: Some("acme-sec".to_string()),
                repo_type: None,
                allow_insecure: None,
                skills: None,
                targets: Some(vec!["claude".to_string()]),
            }),
            ApmDependency::Marketplace(MarketplaceApmDependency {
                name: "code-review".to_string(),
                marketplace: "contoso".to_string(),
                version: Some("^1.0.0".to_string()),
            }),
            ApmDependency::Registry(RegistryApmDependency {
                id: "acme/prompts".to_string(),
                registry: Some("internal".to_string()),
                version: Some("1.2.3".to_string()),
            }),
            ApmDependency::Local(LocalApmDependency {
                path: "./packages/shared".to_string(),
                alias: Some("shared".to_string()),
                skills: Some(vec!["lint".to_string()]),
                targets: None,
            }),
        ];

        let plugin = PublishedPlugin::new(
            "core",
            "skills",
            "1.0.0",
            Some("Core workflow skills"),
            &deps,
        );

        let yaml = render_plugin_manifest(&plugin).unwrap();
        let parsed: Value = serde_yaml_ng::from_str(&yaml).unwrap();
        let apm_deps = parsed["dependencies"]["apm"].as_sequence().unwrap();

        assert_eq!(apm_deps.len(), 5);
        assert_eq!(
            apm_deps[0].as_str(),
            Some("microsoft/apm-sample-package#v1.0.0")
        );
        assert_eq!(
            apm_deps[1]["git"].as_str(),
            Some("https://gitlab.com/acme/repo.git")
        );
        assert_eq!(apm_deps[1]["ref"].as_str(), Some("v2.0"));
        assert_eq!(apm_deps[1]["alias"].as_str(), Some("acme-sec"));
        assert_eq!(apm_deps[2]["name"].as_str(), Some("code-review"));
        assert_eq!(apm_deps[2]["marketplace"].as_str(), Some("contoso"));
        assert_eq!(apm_deps[3]["id"].as_str(), Some("acme/prompts"));
        assert_eq!(apm_deps[4]["path"].as_str(), Some("./packages/shared"));
    }

    #[test]
    fn render_plugin_manifest_rejects_empty_targets_and_empty_strings() {
        let empty_targets = vec![ApmDependency::Git(GitApmDependency {
            git: "owner/repo".to_string(),
            path: None,
            git_ref: None,
            alias: None,
            repo_type: None,
            allow_insecure: None,
            skills: None,
            targets: Some(vec![]),
        })];
        let plugin = PublishedPlugin::new("core", "skills", "1.0.0", None, &empty_targets);
        let err = render_plugin_manifest(&plugin).unwrap_err();
        assert!(matches!(err, ApmError::InvalidDependency { .. }));

        let empty_shorthand = vec![ApmDependency::Shorthand("   ".to_string())];
        let plugin = PublishedPlugin::new("core", "skills", "1.0.0", None, &empty_shorthand);
        let err = render_plugin_manifest(&plugin).unwrap_err();
        assert!(matches!(err, ApmError::InvalidDependency { .. }));

        let local_empty_targets = vec![ApmDependency::Local(LocalApmDependency {
            path: "./shared".to_string(),
            alias: None,
            skills: None,
            targets: Some(vec![]),
        })];
        let plugin = PublishedPlugin::new("core", "skills", "1.0.0", None, &local_empty_targets);
        let err = render_plugin_manifest(&plugin).unwrap_err();
        assert!(matches!(err, ApmError::InvalidDependency { .. }));
    }

    #[test]
    fn merge_root_manifest_scaffolds_when_none() {
        let cfg = sample_publish_cfg();
        let plugins = [PublishedPlugin::new(
            "core",
            "skills",
            "1.0.0",
            Some("Core workflow skills"),
            &[],
        )];

        let merged = merge_root_manifest(None, &cfg, &plugins).unwrap();
        assert!(merged.warnings.is_empty());

        let parsed: Value = serde_yaml_ng::from_str(&merged.yaml).unwrap();
        assert_eq!(parsed["name"].as_str(), Some("my-marketplace"));
        assert_eq!(parsed["version"].as_str(), Some("0.1.0"));
        assert_eq!(
            parsed["marketplace"]["owner"]["name"].as_str(),
            Some("Contoso")
        );
        assert_eq!(
            parsed["marketplace"]["owner"]["email"].as_str(),
            Some("maintainers@contoso.example")
        );
        assert!(parsed["marketplace"]["outputs"]["claude"].is_mapping());
        assert!(parsed["marketplace"]["outputs"]["copilot"].is_mapping());

        let packages = parsed["marketplace"]["packages"].as_sequence().unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0]["name"].as_str(), Some("core"));
        assert_eq!(packages[0]["source"].as_str(), Some("./skills"));
        assert_eq!(packages[0]["version"].as_str(), Some("1.0.0"));
        assert_eq!(
            packages[0]["description"].as_str(),
            Some("Core workflow skills")
        );
    }

    #[test]
    fn merge_root_manifest_warns_on_unknown_agent() {
        let mut cfg = sample_publish_cfg();
        cfg.agents = vec![
            "claude".to_string(),
            "cursor".to_string(),
            "github-copilot".to_string(),
        ];
        let plugins = [PublishedPlugin::new("core", "skills", "1.0.0", None, &[])];

        let merged = merge_root_manifest(None, &cfg, &plugins).unwrap();
        assert_eq!(merged.warnings.len(), 1);
        assert!(merged.warnings[0].contains("cursor"));

        let parsed: Value = serde_yaml_ng::from_str(&merged.yaml).unwrap();
        let outputs = parsed["marketplace"]["outputs"].as_mapping().unwrap();
        assert_eq!(outputs.len(), 2);
        assert!(outputs.contains_key(yaml_str("claude")));
        assert!(outputs.contains_key(yaml_str("copilot")));
    }

    #[test]
    fn merge_root_manifest_preserves_user_keys_and_marketplace_siblings() {
        let existing = r#"name: custom-workspace
version: 2.3.4
description: Hand-authored root manifest
targets:
  - claude
  - copilot
scripts:
  lint: skillet lint
dependencies:
  apm:
    - microsoft/apm-sample-package#v1.0.0
devDependencies:
  apm:
    - acme/dev-tools#main
compilation:
  strategy: distributed
policy:
  strict: true
x-custom:
  team: platform
marketplace:
  sourceBase: https://github.com/contoso/skills
  metadata:
    category: engineering
  build:
    clean: true
  owner:
    name: OldOwner
    email: old@contoso.example
    url: https://contoso.example
  outputs:
    claude: {}
  packages:
    - name: stale
      source: ./stale
      version: 0.0.1
"#;

        let cfg = sample_publish_cfg();
        let plugins = [
            PublishedPlugin::new("zeta", "plugins/zeta", "0.2.0", None, &[]),
            PublishedPlugin::new("alpha", "plugins/alpha", "1.0.0", Some("Alpha"), &[]),
        ];

        let merged = merge_root_manifest(Some(existing), &cfg, &plugins).unwrap();
        let parsed: Value = serde_yaml_ng::from_str(&merged.yaml).unwrap();

        // Top-level user-managed keys preserved
        assert_eq!(parsed["name"].as_str(), Some("custom-workspace"));
        assert_eq!(parsed["version"].as_str(), Some("2.3.4"));
        assert_eq!(
            parsed["description"].as_str(),
            Some("Hand-authored root manifest")
        );
        assert_eq!(parsed["scripts"]["lint"].as_str(), Some("skillet lint"));
        assert_eq!(
            parsed["dependencies"]["apm"][0].as_str(),
            Some("microsoft/apm-sample-package#v1.0.0")
        );
        assert_eq!(
            parsed["devDependencies"]["apm"][0].as_str(),
            Some("acme/dev-tools#main")
        );
        assert_eq!(
            parsed["compilation"]["strategy"].as_str(),
            Some("distributed")
        );
        assert_eq!(parsed["policy"]["strict"].as_bool(), Some(true));
        assert_eq!(parsed["x-custom"]["team"].as_str(), Some("platform"));

        // Non-skillet marketplace siblings and owner.url preserved
        assert_eq!(
            parsed["marketplace"]["sourceBase"].as_str(),
            Some("https://github.com/contoso/skills")
        );
        assert_eq!(
            parsed["marketplace"]["metadata"]["category"].as_str(),
            Some("engineering")
        );
        assert_eq!(
            parsed["marketplace"]["build"]["clean"].as_bool(),
            Some(true)
        );
        assert_eq!(
            parsed["marketplace"]["owner"]["url"].as_str(),
            Some("https://contoso.example")
        );

        // Skillet-managed fields updated
        assert_eq!(
            parsed["marketplace"]["owner"]["name"].as_str(),
            Some("Contoso")
        );
        assert_eq!(
            parsed["marketplace"]["owner"]["email"].as_str(),
            Some("maintainers@contoso.example")
        );
        assert!(parsed["marketplace"]["outputs"]["claude"].is_mapping());
        assert!(parsed["marketplace"]["outputs"]["copilot"].is_mapping());

        let packages = parsed["marketplace"]["packages"].as_sequence().unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0]["name"].as_str(), Some("alpha"));
        assert_eq!(packages[0]["source"].as_str(), Some("./plugins/alpha"));
        assert_eq!(packages[1]["name"].as_str(), Some("zeta"));
        assert_eq!(packages[1]["source"].as_str(), Some("./plugins/zeta"));
    }

    #[test]
    fn merge_root_manifest_normalizes_out_dir_and_rejects_traversal() {
        let cfg = sample_publish_cfg();

        let plugins = [
            PublishedPlugin::new("a", "./skills", "1.0.0", None, &[]),
            PublishedPlugin::new("b", r"plugins\core\skills\", "1.0.0", None, &[]),
        ];
        let merged = merge_root_manifest(None, &cfg, &plugins).unwrap();
        let parsed: Value = serde_yaml_ng::from_str(&merged.yaml).unwrap();
        let packages = parsed["marketplace"]["packages"].as_sequence().unwrap();
        assert_eq!(packages[0]["source"].as_str(), Some("./skills"));
        assert_eq!(
            packages[1]["source"].as_str(),
            Some("./plugins/core/skills")
        );

        let bad_traversal = [PublishedPlugin::new("bad", "../escape", "1.0.0", None, &[])];
        let err = merge_root_manifest(None, &cfg, &bad_traversal).unwrap_err();
        assert!(matches!(err, ApmError::InvalidPackageSource { .. }));

        let bad_mid_traversal = [PublishedPlugin::new(
            "bad",
            "skills/../escape",
            "1.0.0",
            None,
            &[],
        )];
        let err = merge_root_manifest(None, &cfg, &bad_mid_traversal).unwrap_err();
        assert!(matches!(err, ApmError::InvalidPackageSource { .. }));

        let bad_abs = [PublishedPlugin::new("bad", "/abs/path", "1.0.0", None, &[])];
        let err = merge_root_manifest(None, &cfg, &bad_abs).unwrap_err();
        assert!(matches!(err, ApmError::InvalidPackageSource { .. }));
    }

    #[test]
    fn merge_root_manifest_errors_on_malformed_or_non_mapping_yaml() {
        let cfg = sample_publish_cfg();
        let plugins = [PublishedPlugin::new("core", "skills", "1.0.0", None, &[])];

        let err = merge_root_manifest(Some(":::invalid: [yaml"), &cfg, &plugins).unwrap_err();
        assert!(matches!(err, ApmError::InvalidExistingYaml(_)));

        let err = merge_root_manifest(Some("- just\n- a\n- list\n"), &cfg, &plugins).unwrap_err();
        assert_eq!(err, ApmError::RootNotMapping);

        let err = merge_root_manifest(Some("name: test\nmarketplace: not-a-map\n"), &cfg, &plugins)
            .unwrap_err();
        assert_eq!(err, ApmError::MarketplaceNotMapping);
    }
}
