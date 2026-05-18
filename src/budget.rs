//! Token-budget reporting for the `skillet budget` command.

use crate::config;
use crate::lockfile;
use crate::parse::parse_frontmatter;
use crate::tokens::count_tokens;
use crate::workspace;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

// ── Public types ──────────────────────────────────────────────────────────────

/// Per-fragment token entry.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentEntry {
    /// Fragment name (without `.fragment.skill` suffix).
    pub name: String,
    /// Approximate token count for this fragment.
    pub tokens: u32,
}

/// Token budget row for a single skill.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize)]
pub struct BudgetRow {
    /// Skill name.
    pub skill: String,
    /// Tokens for `name` + `description` (discovery cost).
    pub discovery: u32,
    /// Tokens for the full compiled `SKILL.md` (activation cost).
    pub activation: u32,
    /// Activation + tokens for files linked via `ref::` (transitive cost).
    pub transitive: u32,
    /// Per-fragment token entries used by this skill.
    pub fragments: Vec<FragmentEntry>,
}

/// Output format for budget results.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable table.
    #[default]
    Human,
    /// Machine-parseable JSON array.
    Json,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Computes and prints the token budget for the workspace (or a single skill).
///
/// Returns `Ok(())` on success.  Prints to stdout according to `format`.
///
/// # Errors
///
/// Returns an error if the workspace cannot be read.
pub fn run(workspace: &Path, skill_name: Option<&str>, format: OutputFormat) -> Result<()> {
    let config = config::load(workspace)?;
    let skills_src_dir = workspace.join(&config.workspace.skills_src_dir);
    let skills_out_dir = workspace.join(&config.workspace.skills_out_dir);
    let fragments_dir = workspace.join(&config.workspace.fragments_dir);

    let all_sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir)?;
    let lockfile = lockfile::read(workspace)?;

    let targets: Vec<_> = match skill_name {
        Some(name) => all_sources.iter().filter(|s| s.name == name).collect(),
        None => all_sources.iter().collect(),
    };

    let mut rows: Vec<BudgetRow> = Vec::with_capacity(targets.len());
    for source in &targets {
        let row = compute_row(source, &fragments_dir, &lockfile, &config.build.tokenizer)?;
        rows.push(row);
    }

    match format {
        OutputFormat::Human => print_human(&rows),
        OutputFormat::Json => print_json(&rows)?,
    }

    Ok(())
}

// ── Per-skill computation ─────────────────────────────────────────────────────

fn compute_row(
    source: &workspace::SkillSource,
    fragments_dir: &Path,
    lockfile: &lockfile::Lockfile,
    tokenizer: &str,
) -> Result<BudgetRow> {
    let skill_md_path = source.skill_out_dir.join("SKILL.md");
    let compiled = std::fs::read_to_string(&skill_md_path)
        .with_context(|| {
            format!(
                "SKILL.md not found for '{}' — run `skillet build` first",
                source.name
            )
        })?;

    // ── Discovery: name + description from frontmatter ────────────────────────
    let discovery_text = match parse_frontmatter(&compiled) {
        Ok(Some(fm)) => format!(
            "{} {}",
            fm.name.unwrap_or_default(),
            fm.description.unwrap_or_default()
        ),
        _ => String::new(),
    };
    let discovery = count_tokens(&discovery_text, tokenizer);

    // ── Activation: full compiled SKILL.md ────────────────────────────────────
    let activation = count_tokens(&compiled, tokenizer);

    // ── Transitive: activation + linked ref files ─────────────────────────────
    // Scan the .skill source for `ref::path` so we see the unstripped typed refs.
    let source_text = std::fs::read_to_string(&source.source_path).with_context(|| {
        format!("failed to read source '{}'", source.source_path.display())
    })?;
    let ref_tokens: u32 = crate::refs::extract_path_refs(&source_text)
        .into_iter()
        .filter_map(|rel| {
            let path = source.skill_dir.join(&rel);
            std::fs::read_to_string(&path).ok().map(|t| count_tokens(&t, tokenizer))
        })
        .sum();
    let transitive = activation + ref_tokens;

    // ── Fragments: per-fragment token counts from lockfile ────────────────────
    let frag_names = lockfile
        .skills
        .get(&source.name)
        .map(|e| e.fragments_used.as_slice())
        .unwrap_or(&[]);

    let mut fragments = Vec::with_capacity(frag_names.len());
    for frag_name in frag_names {
        let frag_path = fragments_dir.join(format!("{}.fragment.pan", frag_name));
        let tokens = if let Ok(text) = std::fs::read_to_string(&frag_path) {
            count_tokens(&text, tokenizer)
        } else {
            0
        };
        fragments.push(FragmentEntry {
            name: frag_name.clone(),
            tokens,
        });
    }

    Ok(BudgetRow {
        skill: source.name.clone(),
        discovery,
        activation,
        transitive,
        fragments,
    })
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn print_human(rows: &[BudgetRow]) {
    if rows.is_empty() {
        println!("No skills found.");
        return;
    }

    // Determine column widths
    let w_skill = rows.iter().map(|r| r.skill.len()).max().unwrap_or(5).max(5);
    let headers = ["Skill", "Discovery", "Activation", "Transitive", "Fragments"];
    let w_disc = "Discovery".len();
    let w_act = "Activation".len();
    let w_trans = "Transitive".len();
    let w_frags = rows
        .iter()
        .map(|r| {
            if r.fragments.is_empty() {
                0
            } else {
                r.fragments
                    .iter()
                    .map(|f| format!("{}({})", f.name, f.tokens).len())
                    .sum::<usize>()
                    + (r.fragments.len() - 1) * 2 // ", " separators
            }
        })
        .max()
        .unwrap_or(0)
        .max("Fragments".len());

    // Header
    println!(
        "{:<w_skill$}  {:>w_disc$}  {:>w_act$}  {:>w_trans$}  {:<w_frags$}",
        headers[0], headers[1], headers[2], headers[3], headers[4],
        w_skill = w_skill,
        w_disc = w_disc,
        w_act = w_act,
        w_trans = w_trans,
        w_frags = w_frags,
    );

    let sep_width = w_skill + 2 + w_disc + 2 + w_act + 2 + w_trans + 2 + w_frags;
    println!("{}", "─".repeat(sep_width));

    let mut total_disc: u32 = 0;
    let mut total_act: u32 = 0;

    for row in rows {
        let frag_str = if row.fragments.is_empty() {
            String::new()
        } else {
            row.fragments
                .iter()
                .map(|f| format!("{}({})", f.name, f.tokens))
                .collect::<Vec<_>>()
                .join(", ")
        };

        println!(
            "{:<w_skill$}  {:>w_disc$}  {:>w_act$}  {:>w_trans$}  {}",
            row.skill,
            row.discovery,
            row.activation,
            row.transitive,
            frag_str,
            w_skill = w_skill,
            w_disc = w_disc,
            w_act = w_act,
            w_trans = w_trans,
        );

        total_disc += row.discovery;
        total_act += row.activation;
    }

    println!("{}", "─".repeat(sep_width));
    println!(
        "{:<w_skill$}  {:>w_disc$}  {:>w_act$}",
        "TOTAL",
        total_disc,
        total_act,
        w_skill = w_skill,
        w_disc = w_disc,
        w_act = w_act,
    );
}

fn print_json(rows: &[BudgetRow]) -> Result<()> {
    let json = serde_json::to_string_pretty(rows).context("failed to serialise budget as JSON")?;
    println!("{}", json);
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;
    use crate::init;
    use crate::workspace::SkillSource;
    use std::fs;
    use tempfile::TempDir;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn init_workspace(dir: &std::path::Path) {
        init::run(dir, false).expect("init failed");
    }

    fn make_skill(dir: &std::path::Path, name: &str, description: &str, body: &str) {
        let skill_dir = dir.join("src/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let source = format!(
            "---\nname: {name}\ndescription: \"{description}\"\n---\n\n{body}",
        );
        fs::write(skill_dir.join(format!("{name}.pan")), source).unwrap();
    }

    fn built_source(dir: &std::path::Path, name: &str) -> SkillSource {
        let skill_dir = dir.join("src/skills").join(name);
        let skill_out_dir = dir.join("skills").join(name);
        SkillSource {
            name: name.to_string(),
            skill_dir: skill_dir.clone(),
            skill_out_dir,
            source_path: skill_dir.join(format!("{name}.pan")),
        }
    }

    // ── unit tests ────────────────────────────────────────────────────────────

    #[test]
    fn compute_row_returns_nonzero_activation_for_nonempty_skill() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        make_skill(tmp.path(), "alpha", "does alpha things", "## Usage\nrun alpha\n");
        build::run(tmp.path(), Some("alpha"), &Default::default()).unwrap();

        // Act
        let lockfile = lockfile::read(tmp.path()).unwrap();
        let source = built_source(tmp.path(), "alpha");
        let row = compute_row(
            &source,
            &tmp.path().join("src/skills/_fragments"),
            &lockfile,
            "cl100k_base",
        )
        .unwrap();

        // Assert
        assert!(row.activation > 0, "activation should be positive");
        assert!(row.discovery > 0, "discovery should be positive");
        assert!(row.transitive >= row.activation, "transitive >= activation");
    }

    #[test]
    fn discovery_is_less_than_activation_for_skill_with_body() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        make_skill(
            tmp.path(),
            "beta",
            "short desc",
            "## Long body\n".repeat(20).as_str(),
        );
        build::run(tmp.path(), Some("beta"), &Default::default()).unwrap();

        // Act
        let lockfile = lockfile::read(tmp.path()).unwrap();
        let source = built_source(tmp.path(), "beta");
        let row = compute_row(
            &source,
            &tmp.path().join("src/skills/_fragments"),
            &lockfile,
            "cl100k_base",
        )
        .unwrap();

        // Assert
        assert!(
            row.discovery < row.activation,
            "discovery ({}) should be < activation ({})",
            row.discovery,
            row.activation
        );
    }

    #[test]
    fn fragment_tokens_are_populated_when_fragment_is_used() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        let frags_dir = tmp.path().join("src/skills/_fragments");
        fs::create_dir_all(&frags_dir).unwrap();
        fs::write(
            frags_dir.join("note.fragment.pan"),
            "## Note\nfragment body\n",
        )
        .unwrap();
        make_skill(tmp.path(), "gamma", "uses a fragment", "{{> note }}\n");
        build::run(tmp.path(), Some("gamma"), &Default::default()).unwrap();

        // Act
        let lockfile = lockfile::read(tmp.path()).unwrap();
        let source = built_source(tmp.path(), "gamma");
        let row = compute_row(&source, &frags_dir, &lockfile, "cl100k_base").unwrap();

        // Assert
        assert_eq!(row.fragments.len(), 1);
        assert_eq!(row.fragments[0].name, "note");
        assert!(row.fragments[0].tokens > 0);
    }

    #[test]
    fn transitive_equals_activation_when_no_refs() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        make_skill(tmp.path(), "delta", "no refs here", "## Body\ncontent\n");
        build::run(tmp.path(), Some("delta"), &Default::default()).unwrap();

        // Act
        let lockfile = lockfile::read(tmp.path()).unwrap();
        let source = built_source(tmp.path(), "delta");
        let row = compute_row(
            &source,
            &tmp.path().join("src/skills/_fragments"),
            &lockfile,
            "cl100k_base",
        )
        .unwrap();

        // Assert
        assert_eq!(
            row.transitive, row.activation,
            "no ref:: links → transitive == activation"
        );
    }

    #[test]
    fn compute_row_errors_when_skill_md_missing() {
        // Arrange
        let tmp = TempDir::new().unwrap();
        init_workspace(tmp.path());
        make_skill(tmp.path(), "epsilon", "unbuilt", "## Body\n");
        // deliberately do NOT call build

        // Act
        let lockfile = lockfile::read(tmp.path()).unwrap();
        let source = built_source(tmp.path(), "epsilon");
        let result = compute_row(
            &source,
            &tmp.path().join("src/skills/_fragments"),
            &lockfile,
            "cl100k_base",
        );

        // Assert
        assert!(result.is_err(), "should error when SKILL.md is missing");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("skillet build"), "error should mention skillet build");
    }
}
