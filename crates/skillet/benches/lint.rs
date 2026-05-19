#![allow(missing_docs)]
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use skillet::{
    build,
    lint::{self, LintOptions, OutputFormat},
};
use std::fs;
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

fn setup_workspace(n_skills: usize) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

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
        build::run(dir, Some(&name), &Default::default()).unwrap();
    }

    tmp
}

fn bench_lint_scaling(c: &mut Criterion) {
    const SIZES: &[usize] = &[10, 20, 50];

    let workspaces: Vec<(usize, TempDir)> =
        SIZES.iter().map(|&n| (n, setup_workspace(n))).collect();

    let mut group = c.benchmark_group("lint");
    group.sample_size(20);

    // mean_ns[i] accumulates per-sample nanos for SIZES[i]
    let mut mean_ns: Vec<(usize, Vec<f64>)> = SIZES.iter().map(|&n| (n, Vec::new())).collect();

    for ((n, tmp), (_, samples)) in workspaces.iter().zip(mean_ns.iter_mut()) {
        let opts = LintOptions::new(false, false, OutputFormat::Silent);
        group.bench_with_input(BenchmarkId::new("skills", n), n, |b, _| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    lint::run(black_box(tmp.path()), None, &opts).unwrap();
                }
                let elapsed = start.elapsed();
                samples.push(elapsed.as_nanos() as f64 / iters as f64);
                elapsed
            });
        });
    }

    group.finish();

    // ── Summary table ─────────────────────────────────────────────────────────
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
