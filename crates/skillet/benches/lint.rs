#![allow(missing_docs)]
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rayon::prelude::*;
use sha2::Digest;
use skillet::{
    config::SkilletConfig,
    lint::{pipeline, rules, LintContext},
    lockfile,
    tokens::count_tokens,
    workspace,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tempfile::TempDir;
use walkdir::WalkDir;

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
    use skillet::compile::{compile, CompileContext, SourceUnit};
    use std::collections::{HashMap, HashSet};

    let skills_src_dir = dir.join(&cfg.workspace.skills_src_dir);
    let skills_out_dir = dir.join(&cfg.workspace.skills_out_dir);
    let source = workspace::discover_skills(&skills_src_dir, &skills_out_dir)
        .unwrap()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap();

    let source_content = fs::read_to_string(&source.source_path).unwrap();
    let ctx = CompileContext {
        source: SourceUnit {
            name: source.name.clone(),
            path: source.source_path.to_string_lossy().to_string(),
            content: source_content,
        },
        fragments: HashMap::new(),
        known_files: HashSet::new(),
        known_commands: HashSet::new(),
        known_skills: HashSet::new(),
        vars: cfg.vars.clone(),
        env: cfg.env.clone(),
        tokenizer: cfg.build.tokenizer.clone(),
    };
    let result = compile(&ctx).unwrap();
    fs::create_dir_all(&source.skill_out_dir).unwrap();
    fs::write(source.skill_out_dir.join("SKILL.md"), &result.output).unwrap();
}

/// Builds LintContext from workspace (same as CLI does).
fn build_lint_context(
    all_sources: &[workspace::SkillSource],
    fragments_dir: &Path,
    cfg: &SkilletConfig,
) -> LintContext {
    let mut ctx = LintContext::default();

    for src in all_sources {
        let mut files = HashSet::new();
        if src.skill_dir.exists() {
            for entry in WalkDir::new(&src.skill_dir)
                .min_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.path().is_file() {
                    if let Ok(rel) = entry.path().strip_prefix(&src.skill_dir) {
                        files.insert(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        ctx.skill_files.insert(src.name.clone(), files);
        ctx.known_skill_dirs.insert(src.name.clone());
    }

    for src in all_sources {
        let path = src.skill_out_dir.join("SKILL.md");
        if let Ok(text) = fs::read_to_string(&path) {
            let hash = format!(
                "sha256:{}",
                hex::encode(sha2::Sha256::digest(text.as_bytes()))
            );
            ctx.activation_tokens
                .insert(src.name.clone(), count_tokens(&text, &cfg.build.tokenizer));
            ctx.compiled_hashes.insert(src.name.clone(), hash);
            ctx.compiled_texts.insert(src.name.clone(), text);
        }
    }

    if fragments_dir.exists() {
        if let Ok(entries) = fs::read_dir(fragments_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let fname = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let frag_name = match fname.strip_suffix(".fragment.pan") {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                ctx.fragment_names.push(frag_name.clone());
                if let Ok(content) = fs::read_to_string(&path) {
                    let hash = format!(
                        "sha256:{}",
                        hex::encode(sha2::Sha256::digest(content.as_bytes()))
                    );
                    let tokens = count_tokens(&content, &cfg.build.tokenizer);
                    ctx.fragment_hashes.insert(frag_name.clone(), hash);
                    ctx.fragment_tokens.insert(frag_name, tokens);
                }
            }
        }
    }

    ctx
}

/// Runs lint inline (same as CLI orchestration, minus rendering).
fn run_lint(workspace_path: &Path, cfg: &SkilletConfig) {
    let skills_src_dir = workspace_path.join(&cfg.workspace.skills_src_dir);
    let skills_out_dir = workspace_path.join(&cfg.workspace.skills_out_dir);
    let fragments_dir = workspace_path.join(&cfg.workspace.fragments_dir);
    let lf = lockfile::read(workspace_path).unwrap();
    let all_sources = workspace::discover_skills(&skills_src_dir, &skills_out_dir).unwrap();

    let ctx = build_lint_context(&all_sources, &fragments_dir, cfg);

    let inputs: Vec<pipeline::SourceInput> = all_sources
        .iter()
        .map(|src| {
            let content = fs::read_to_string(&src.source_path).unwrap_or_default();
            let mut reference_docs = Vec::new();
            let ref_dir = src.skill_dir.join("reference");
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
                name: src.name.clone(),
                source_path: src.source_path.clone(),
                skill_dir: src.skill_dir.clone(),
                skill_out_dir: src.skill_out_dir.clone(),
                content,
                reference_docs,
            }
        })
        .collect();

    let source_files = pipeline::scan_sources(&inputs, &cfg.build.tokenizer);
    let skill_names: Vec<&str> = all_sources.iter().map(|s| s.name.as_str()).collect();
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
