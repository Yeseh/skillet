#![allow(missing_docs)]
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rayon::prelude::*;
use sha2::Digest;
use skillet::{
    config::SkilletConfig,
    lint::{pipeline, rules, LintContext},
    lockfile,
    tokens::count_tokens,
    workspace::Workspace,
};
use std::fs;
use std::path::Path;
use std::time::Instant;
use tempfile::TempDir;

static CONFIG_TOML: &str = r#"
[workspace]
skills_src_dir = "src/skills"
skills_out_dir = "skills"
fragments_dir = "src/skills/_fragments"

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500
allowed_commands = []
disable = []

[build]
tokenizer = "cl100k_base"
verify_urls = false

[vars]
project_name = "bench-project"

[env]
CI = {default="true"}
"#;

/// Minimal build for bench setup (inline version of test_support::build_workspace).
fn build_skill(dir: &Path, name: &str, cfg: &SkilletConfig) {
    use skillet::compiler::{compile_pan, render_fragments, CompileContext, PanSource};
    use std::collections::HashSet;

    let ws = Workspace::resolve(dir, cfg).unwrap();
    let skill = ws.skills.iter().find(|s| s.name == name).unwrap();

    let source_content = fs::read_to_string(&skill.source_path).unwrap();
    let rendered_fragments = render_fragments(&std::collections::HashMap::new());
    let known_skills = HashSet::new();
    let known_files = HashSet::new();
    let known_commands = HashSet::new();
    let known_agents = HashSet::new();
    let ctx = CompileContext {
        source: PanSource::new(source_content, Some(skill.source_path.clone())),
        artifact_name: skill.name.clone(),
        fragments: &rendered_fragments,
        vars: &cfg.vars,
        env: &cfg.env,
        known_skills: &known_skills,
        known_references: &known_files,
        known_commands: &known_commands,
        known_agents: &known_agents,
        tokenizer: &cfg.build.tokenizer,
    };
    let result = compile_pan(&ctx).unwrap();
    fs::create_dir_all(&skill.target_dir).unwrap();
    fs::write(skill.target_dir.join("SKILL.md"), &result.output).unwrap();
}

/// Builds LintContext from resolved workspace (same as CLI does).
fn build_lint_context(ws: &Workspace, cfg: &SkilletConfig) -> LintContext {
    let mut ctx = LintContext::default();

    for skill in &ws.skills {
        let files = ws.get_references_for_skill(skill);
        ctx.skill_files.insert(skill.name.clone(), files);
        ctx.known_skill_dirs.insert(skill.name.clone());
    }

    for skill in &ws.skills {
        let path = skill.target_dir.join("SKILL.md");
        if let Ok(text) = fs::read_to_string(&path) {
            let hash = format!(
                "sha256:{}",
                hex::encode(sha2::Sha256::digest(text.as_bytes()))
            );
            ctx.activation_tokens.insert(
                skill.name.clone(),
                count_tokens(&text, &cfg.build.tokenizer),
            );
            ctx.compiled_hashes.insert(skill.name.clone(), hash);
            ctx.compiled_texts.insert(skill.name.clone(), text);
        }
    }

    ctx.fragment_names = ws
        .fragment_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    ctx.fragment_hashes = ws.fragment_hashes.clone();
    ctx.fragment_tokens = ws.fragment_tokens.clone();

    ctx
}

/// Runs lint inline (same as CLI orchestration, minus rendering).
fn run_lint(workspace_path: &Path, cfg: &SkilletConfig) {
    let ws = Workspace::resolve(workspace_path, cfg).unwrap();
    let lf = lockfile::read(workspace_path).unwrap();

    let ctx = build_lint_context(&ws, cfg);

    let inputs: Vec<pipeline::SourceInput> = ws
        .skills
        .iter()
        .map(|skill| {
            let content = fs::read_to_string(&skill.source_path).unwrap_or_default();
            let mut reference_docs = Vec::new();
            let ref_dir = skill.src_dir.join("reference");
            if ref_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&ref_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            if let Ok(text) = fs::read_to_string(&path) {
                                reference_docs.push((path, text));
                            }
                        }
                    }
                }
            }
            pipeline::SourceInput {
                name: skill.name.clone(),
                source_path: skill.source_path.clone(),
                skill_dir: skill.src_dir.clone(),
                skill_out_dir: skill.target_dir.clone(),
                content,
                reference_docs,
            }
        })
        .collect();

    let source_files = pipeline::scan_sources(&inputs, &cfg.build.tokenizer);
    let skill_names: Vec<&str> = ws.skills.iter().map(|s| s.name.as_str()).collect();
    let (source_files, _) = pipeline::extract_refs(source_files, &skill_names);

    let _diags: Vec<skillet::lint::Diagnostic> = source_files
        .par_iter()
        .filter(|sf| matches!(sf.file_type, pipeline::SourceFileType::Skill))
        .flat_map(|sf| {
            let mut diags = Vec::new();
            diags.extend(rules::invalid_frontmatter::check(sf, cfg));
            diags.extend(rules::stale_refs::check(sf, cfg, &ctx));
            diags.extend(rules::markdown_links::check(sf, cfg, &ctx));
            diags.extend(rules::untyped_backtick::check(sf));
            diags.extend(rules::stale_build::check(sf, &lf, &ctx));
            diags.extend(rules::oversized::check_skill(sf, cfg, &lf, &ctx));
            diags.extend(rules::oversized::check_description(sf, cfg));
            diags
        })
        .collect();
}

fn setup_workspace(n_skills: usize) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let cfg = SkilletConfig::default();

    fs::write(dir.join("skillet.toml"), CONFIG_TOML).unwrap();
    fs::create_dir_all(dir.join("src/skills")).unwrap();
    fs::create_dir_all(dir.join("skills")).unwrap();
    fs::create_dir_all(dir.join("src/skills/_fragments")).unwrap();

    for i in 0..n_skills {
        let name = format!("skill-{i:03}");
        let skill_src = dir.join("src/skills").join(&name);
        fs::create_dir_all(&skill_src).unwrap();

        let body = format!(
            "This skill handles operation {i}. It performs task {i} on the workspace.\n\
             Use this when you need to invoke function {i}. Returns result {i}.\n\
             Only applies to context {i}. Never mix with skill {}.\n",
            (i + 1) % n_skills
        );
        let content = format!(
            "---\nname: {name}\ndescription: Performs operation {i} on the workspace.\n---\n\n\
             # {name}\n\n{body}"
        );
        fs::write(skill_src.join(format!("{name}.pan")), &content).unwrap();
        build_skill(dir, &name, &cfg);
    }

    tmp
}

fn bench_lint_scaling(c: &mut Criterion) {
    const SIZES: &[usize] = &[10, 20, 50];
    let cfg = SkilletConfig::default();

    let workspaces: Vec<(usize, TempDir)> =
        SIZES.iter().map(|&n| (n, setup_workspace(n))).collect();

    let mut group = c.benchmark_group("lint");
    group.sample_size(20);

    let mut mean_ns: Vec<(usize, Vec<f64>)> = SIZES.iter().map(|&n| (n, Vec::new())).collect();

    for ((n, tmp), (_, samples)) in workspaces.iter().zip(mean_ns.iter_mut()) {
        group.bench_with_input(BenchmarkId::new("skills", n), n, |b, _| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_lint(black_box(tmp.path()), &cfg);
                }
                let elapsed = start.elapsed();
                samples.push(elapsed.as_nanos() as f64 / iters as f64);
                elapsed
            });
        });
    }

    group.finish();

    println!("\n┌────────┬───────────┬───────────┐");
    println!("│ skills │   mean    │ per skill │");
    println!("├────────┼───────────┼───────────┤");
    for (n, samples) in &mean_ns {
        let mean_ms = samples.iter().sum::<f64>() / samples.len() as f64 / 1_000_000.0;
        let per_ms = mean_ms / *n as f64;
        println!("│ {:>6} │ {:>7.2}ms │ {:>7.3}ms │", n, mean_ms, per_ms);
    }
    println!("└────────┴───────────┴───────────┘");
}

criterion_group!(benches, bench_lint_scaling);
criterion_main!(benches);
