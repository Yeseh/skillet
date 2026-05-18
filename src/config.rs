use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub skills_dir: String,
    pub fragments_dir: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LintConfig {
    pub max_activation_tokens: u32,
    pub max_discovery_tokens: u32,
    pub max_fragment_tokens: u32,
    pub allowed_commands: Vec<String>,
    pub disable: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    pub tokenizer: String,
    pub verify_urls: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnvVar {
    pub default: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkilletConfig {
    pub workspace: WorkspaceConfig,
    pub lint: LintConfig,
    pub build: BuildConfig,
    pub vars: BTreeMap<String, String>,
    pub env: BTreeMap<String, EnvVar>,
}

impl Default for SkilletConfig {
    fn default() -> Self {
        let mut vars = BTreeMap::new();
        vars.insert("project_name".to_string(), "my-project".to_string());

        let mut env = BTreeMap::new();
        env.insert(
            "CI".to_string(),
            EnvVar {
                default: "false".to_string(),
            },
        );
        env.insert(
            "TEAM_NAME".to_string(),
            EnvVar {
                default: "engineering".to_string(),
            },
        );

        SkilletConfig {
            workspace: WorkspaceConfig {
                skills_dir: "skills".to_string(),
                fragments_dir: "skills/_fragments".to_string(),
            },
            lint: LintConfig {
                max_activation_tokens: 4000,
                max_discovery_tokens: 100,
                max_fragment_tokens: 500,
                allowed_commands: vec![
                    "playwright".to_string(),
                    "docker".to_string(),
                    "kubectl".to_string(),
                ],
                disable: vec![],
            },
            build: BuildConfig {
                tokenizer: "cl100k_base".to_string(),
                verify_urls: false,
            },
            vars,
            env,
        }
    }
}

pub fn render_default() -> anyhow::Result<String> {
    let cfg = SkilletConfig::default();
    Ok(toml::to_string_pretty(&cfg)?)
}
