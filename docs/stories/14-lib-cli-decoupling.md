# 14 — Decouple lib crate from filesystem I/O

Restructure the `skillet` lib crate so it contains only pure logic. The `skillet-cli` crate becomes the sole owner of filesystem access, workspace discovery, and output formatting.

See ADR-0004 (`docs/adr/0004-lib-cli-io-boundary.md`) for the decision record.

Each step must compile and pass `cargo test --workspace` before the next begins.

---

## Step 1 — Move `config::load` to CLI

The load function reads `skillet.toml` from disk. The types are domain logic and stay in lib.

- [ ] Remove `pub fn load(workspace: &Path) -> Result<SkilletConfig>` from `crates/skillet/src/config.rs`
- [ ] Remove `use std::fs` from `config.rs`; all remaining imports should be `serde`, `toml`, `std::collections`
- [ ] Add `crates/skillet-cli/src/config.rs` exporting `pub fn load(path: &Path) -> Result<SkilletConfig>` — same implementation, moved verbatim
- [ ] Update `new::run` signature: replace `workspace: &Path` resolution of `skills_src_dir` with a direct `skills_src_dir: &Path` parameter — caller resolves it
- [ ] Update `build::run`, `lint::run`, `check::run`, `budget::run`: add `config: &SkilletConfig` parameter; remove internal `config::load` calls
- [ ] Update `main.rs`: call `skillet_cli::config::load(&cwd)` at the top of each command arm, pass result into lib functions
- [ ] `cargo test --workspace` passes

---

## Step 2 — Reorganize lib modules around domain concepts

Remove command-mirrored modules; consolidate into domain modules.

### New lib module structure

| Module | Replaces | Responsibility |
|---|---|---|
| `config` | `config` | Types only: `SkilletConfig`, `WorkspaceConfig`, `LintConfig`, `BuildConfig`, `EnvVar` |
| `compile` | `build` | `SourceUnit`, `CompileContext`, `CompileResult`, compilation pipeline |
| `lint` | `lint/` | `LintContext`, `LintResult`, `LintDiagnostic`, all lint rules |
| `budget` | `budget` | `BudgetReport`, `SkillBudget`, budget calculations from lockfile data |
| `lockfile` | `lockfile` | `Lockfile`, `LockEntry`, `SkillRefs`, freshness check and diff logic |
| `tokens` | `tokens` | Token counting — unchanged |
| `parse` | `parse` | `.pan` parsing — unchanged |
| `refs` | `refs` | Ref extraction — unchanged |
| `skill` | `skill` | Bundled skill registry — unchanged (no filesystem access) |

### Modules removed from lib

- [ ] Delete `crates/skillet/src/init.rs` — all logic is I/O; no pure residual worth keeping in lib
- [ ] Delete `crates/skillet/src/new.rs` — move `scaffold_content(name: &str) -> String` into `compile` module
- [ ] Delete `crates/skillet/src/check.rs` — staleness comparison logic folds into `lockfile::is_fresh`
- [ ] Delete `crates/skillet/src/workspace.rs` — moves to CLI in Step 3
- [ ] Delete `crates/skillet/src/net/` — URL verification moves to CLI in Step 3

### OutputFormat enums removed from lib

- [ ] Remove `build::OutputFormat`, `lint::OutputFormat`, `check::OutputFormat`, `budget::OutputFormat` — these are presentation concerns; CLI will own formatting
- [ ] Remove `owo-colors` dependency from `crates/skillet/Cargo.toml` — no text rendering in lib
- [ ] Remove `rustls` and `webpki-roots` from `crates/skillet/Cargo.toml` — network I/O moves to CLI

### Rename `build` → `compile`

- [ ] Rename `crates/skillet/src/build.rs` to `compile.rs`
- [ ] Rename the public module in `lib.rs` accordingly
- [ ] Update all internal cross-module references

- [ ] `cargo test --workspace` passes

---

## Step 3 — Move workspace discovery and network I/O to CLI

### Workspace module in CLI

- [ ] Create `crates/skillet-cli/src/workspace.rs` with:

```rust
pub struct WorkspaceSkill {
    pub name: String,
    pub source_path: PathBuf,
    pub skill_dir: PathBuf,
    pub skill_out_dir: PathBuf,
}

pub fn discover_skills(src_dir: &Path, out_dir: &Path) -> Result<Vec<WorkspaceSkill>>
pub fn read_source(skill: &WorkspaceSkill) -> Result<String>
pub fn read_fragments(fragments_dir: &Path) -> Result<HashMap<String, String>>
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()>
```

- [ ] Move `discover_skills` and `copy_dir_recursive` implementations verbatim from `crates/skillet/src/workspace.rs`
- [ ] Add `read_source` — reads `skill.source_path` and returns content as `String`
- [ ] Add `read_fragments` — walks `fragments_dir`, reads each `.fragment.pan` file, returns `HashMap<fragment_name, content>`
- [ ] Add `walkdir` to `crates/skillet-cli/Cargo.toml`; remove it from `crates/skillet/Cargo.toml`

### Network module in CLI

- [ ] Create `crates/skillet-cli/src/net.rs` (or inline into the build command handler) for HTTP URL verification
- [ ] Add `rustls` and `webpki-roots` to `crates/skillet-cli/Cargo.toml`; remove from `crates/skillet/Cargo.toml`
- [ ] URL checking is invoked by the CLI after `compile::compile()` returns — `CompileResult` carries `urls: Vec<String>` for deferred verification

### Init and adopt logic in CLI

- [ ] Create `crates/skillet-cli/src/init.rs` with the full `init` command handler: create dirs, write `skillet.toml`, run adoption
- [ ] The adoption renaming convention (`.md` → `.pan` for `reference/` files, copy other subdirs verbatim) lives in CLI — it is pure I/O with a naming rule, not domain logic
- [ ] `skillet-cli::init` uses `SkilletConfig::default().to_toml()` (from lib) to generate the config content

### CLI main.rs assembly pattern

Each command arm in `main.rs` follows this sequence:
1. Load config: `cli::config::load(&cwd)`
2. Discover sources: `cli::workspace::discover_skills(...)`
3. Read content: `cli::workspace::read_source(...)` + `read_fragments(...)`
4. Assemble `CompileContext` / `LintContext`
5. Call lib function
6. Write output to disk (for build) or format result for user

- [ ] `cargo test --workspace` passes

---

## Step 4 — Strip I/O from lib function signatures

### New public lib API

**`compile` module:**
```rust
pub struct SourceUnit {
    pub name: String,
    pub content: String,
}

pub struct CompileContext {
    pub source: SourceUnit,
    pub fragments: HashMap<String, String>,  // fragment name → content
    pub config: BuildConfig,
}

pub struct CompileResult {
    pub name: String,
    pub output: String,           // compiled SKILL.md content
    pub token_count: u32,
    pub warnings: Vec<String>,
    pub urls: Vec<String>,        // URLs found; CLI verifies if verify_urls = true
}

pub fn compile(ctx: &CompileContext) -> Result<CompileResult>
pub fn scaffold_content(name: &str) -> String
```

**`lint` module:**
```rust
pub struct LintContext {
    pub source: SourceUnit,
    pub fragments: HashMap<String, String>,
    pub config: LintConfig,
}

pub struct LintDiagnostic { /* severity, rule_id, message, line, col */ }
pub struct LintResult {
    pub name: String,
    pub diagnostics: Vec<LintDiagnostic>,
}

pub fn lint(ctx: &LintContext) -> Result<LintResult>
```

**`budget` module:**
```rust
pub fn compute_budget(entries: &[SkillEntry]) -> BudgetReport
pub struct BudgetReport { pub skills: Vec<SkillBudget> }
pub struct SkillBudget { pub name: String, pub discovery: u32, pub activation: u32, pub transitive: u32 }
```

**`lockfile` module:**
```rust
pub fn is_fresh(entry: &SkillEntry, source_hash: &str, output_hash: &str) -> bool
pub fn load(path: &Path) -> Result<Lockfile>   // stays in lib — lockfile is domain data
pub fn save(path: &Path, lockfile: &Lockfile) -> Result<()>
```

Note: `lockfile::load` and `lockfile::save` may retain `&Path` since reading/writing the lockfile is tightly coupled to its content semantics. If this is contentious, move to CLI in the same pass.

### Verification

- [ ] `grep -r "use std::fs\|use std::io\|walkdir" crates/skillet/src/` returns no matches (except `lockfile.rs` if retained)
- [ ] No `&Path` or `PathBuf` in any public function signature outside `lockfile`
- [ ] No `json: bool`, `OutputFormat`, or `println!` / `eprintln!` in lib
- [ ] `serde_json` moved to `skillet-cli/Cargo.toml` (lib no longer serializes output)
- [ ] `owo-colors` removed from `skillet/Cargo.toml`
- [ ] End-to-end: `skillet build`, `skillet lint`, `skillet check`, `skillet budget` all produce identical output to pre-refactor
- [ ] `cargo test --workspace` passes

---

## Step 5 — Migrate integration tests to CLI

### Remove disk dependencies from lib tests

- [ ] Remove `tempfile` from `crates/skillet/Cargo.toml` `[dev-dependencies]`
- [ ] Rewrite all lib tests as pure unit tests: construct `SourceUnit` / `CompileContext` inline, assert on result fields
- [ ] Remove any `use std::fs` or `TempDir` from `crates/skillet/src/**`

### Add integration tests to CLI

- [ ] Create `crates/skillet-cli/tests/` with a `fixtures/` subdirectory containing reusable `.pan` sources and expected compiled outputs
- [ ] Add `tempfile` to `crates/skillet-cli/Cargo.toml` `[dev-dependencies]`
- [ ] Integration test coverage:
  - [ ] `init` creates expected directory structure and `skillet.toml`
  - [ ] `init --adopt` copies `SKILL.md` → `{name}.pan`, renames `reference/*.md` → `reference/*.pan`
  - [ ] `build` with fragment include resolves and inlines correctly
  - [ ] `build` with missing fragment returns error
  - [ ] `build` with `var::` substitution produces correct output
  - [ ] `build` updates `skillet.lock` with correct hashes and token counts
  - [ ] `lint` reports expected diagnostics for an oversized skill
  - [ ] `check` returns stale when source changes after build
  - [ ] `check` returns fresh when lockfile matches
  - [ ] `--format json` on all commands produces valid JSON matching result struct shapes

### Final state

- [ ] `cargo test --workspace` passes with zero disk access in lib tests
- [ ] `cargo clippy --workspace -- -D warnings` clean
- [ ] `grep -r "use std::fs\|walkdir\|tempfile" crates/skillet/src/` returns no matches
