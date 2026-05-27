# Skillet Library — Remaining I/O Remediation Plan

## Goal

Remove all filesystem, network, and environment I/O from the `skillet` library crate so it becomes a pure domain crate. All I/O moves to the CLI (or is injected by callers).

---

## Current State

| Module | I/O Type | Details |
|--------|----------|---------|
| `lint/pipeline.rs` | `std::fs::read_to_string`, `read_dir`, `is_file` | Phase 1 reads `.pan` files and `reference/` docs |
| `lint/rules/stale_build.rs` | `hash_file`, `exists` | Hashes source/compiled/fragments, checks SKILL.md exists |
| `lint/rules/stale_refs.rs` | `exists`, `is_dir`, `is_on_path` | Checks ref targets exist on disk and PATH |
| `lint/rules/markdown_links.rs` | `exists` | Checks markdown link targets resolve |
| `lint/rules/oversized.rs` | `read_to_string`, `read_dir` | Reads SKILL.md and fragments for token counting |
| `lint/rules/unused_fragment.rs` | `exists`, `read_dir` | Lists fragment files on disk |
| `lint/rules/duplication.rs` | `read_to_string` | Reads compiled SKILL.md for each skill |
| `workspace.rs` | `WalkDir`, `read_to_string`, `read`, `fs::copy`, `create_dir_all` | Discovery, hashing, copying |
| `lockfile.rs` | `read_to_string`, `fs::write` | `read()`/`write()` convenience functions |
| `net/url_verify.rs` | `TcpStream`, `rustls` | URL verification over network |

---

## Design: `LintContext`

Introduce a single struct that carries all pre-loaded data rules need. The CLI builds it; the library consumes it.

```rust
/// Pre-loaded workspace state for lint rule execution.
/// Built by the CLI; consumed by pure rule functions.
pub struct LintContext {
    /// Files known to exist relative to each skill dir.
    /// Key: skill name, Value: set of relative paths.
    pub skill_files: HashMap<String, HashSet<String>>,

    /// Commands confirmed present on PATH.
    pub known_commands: HashSet<String>,

    /// Skill directory names that exist in the workspace.
    pub known_skill_dirs: HashSet<String>,

    /// SHA-256 hash of each compiled SKILL.md (key: skill name).
    pub compiled_hashes: HashMap<String, String>,

    /// Full text of each compiled SKILL.md (key: skill name).
    /// Needed by duplication detection.
    pub compiled_texts: HashMap<String, String>,

    /// SHA-256 hash of each fragment file (key: fragment name).
    pub fragment_hashes: HashMap<String, String>,

    /// Token count per fragment (key: fragment name).
    pub fragment_tokens: HashMap<String, u32>,

    /// All fragment names present in the fragments directory.
    pub fragment_names: Vec<String>,

    /// Activation token count per skill from lockfile (key: skill name).
    /// Used by oversized rule when lockfile data is available.
    pub activation_tokens: HashMap<String, u32>,
}
```

---

## Step-by-Step Execution

### Step 1 — Make `pipeline::scan_sources` accept pre-loaded content

**Current signature:**
```rust
pub fn scan_sources(sources: &[SkillSource], tokenizer: &str) -> Vec<SourceFile>
```

**New signature:**
```rust
pub struct SourceInput {
    pub name: String,
    pub source_path: PathBuf,
    pub skill_dir: PathBuf,
    pub skill_out_dir: PathBuf,
    pub content: String,
    pub reference_docs: Vec<(PathBuf, String)>,
}

pub fn scan_sources(inputs: &[SourceInput], tokenizer: &str) -> Vec<SourceFile>
```

**Changes:**
- Remove `std::fs::read_to_string` and `read_dir` from `pipeline.rs`
- Remove `read_and_scan` and `scan_skill_files` private functions
- The function hashes in-memory content, counts tokens, parses frontmatter — all pure
- CLI pre-reads all files and builds `SourceInput` structs

**Files touched:** `crates/skillet/src/lint/pipeline.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 2 — Introduce `LintContext` and update `stale_build`

**Changes to `stale_build::check`:**
```rust
// Before:
pub fn check(source: &SourceFile, fragments_dir: &Path, lockfile: &Lockfile) -> Vec<Diagnostic>

// After:
pub fn check(source: &SourceFile, lockfile: &Lockfile, ctx: &LintContext) -> Vec<Diagnostic>
```

- Replace `output_path.exists()` → `ctx.compiled_hashes.contains_key(&source.name)`
- Replace `hash_file(&source.source_path)` → use `source.source_hash` (already computed in Phase 1)
- Replace `hash_file(&frag_path)` → `ctx.fragment_hashes.get(frag_name)`
- Replace `hash_file(&output_path)` → `ctx.compiled_hashes.get(&source.name)`

**Files touched:** `crates/skillet/src/lint/rules/stale_build.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 3 — Update `stale_refs`

**Changes to `stale_refs::check`:**
```rust
// Before:
pub fn check(source: &SourceFile, config: &SkilletConfig, all_sources: &[SkillSource], skills_src_dir: &Path) -> Vec<Diagnostic>

// After:
pub fn check(source: &SourceFile, config: &SkilletConfig, ctx: &LintContext) -> Vec<Diagnostic>
```

- Replace `source.skill_dir.join(&tr.value).exists()` → `ctx.skill_files[&source.name].contains(&tr.value)`
- Replace `workspace::is_on_path(cmd)` → `ctx.known_commands.contains(cmd)`
- Replace `skills_src_dir.join(&tr.value).is_dir()` → `ctx.known_skill_dirs.contains(&tr.value)`
- `all_sources` name check stays (already pure — iterates in-memory slice)

**Files touched:** `crates/skillet/src/lint/rules/stale_refs.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 4 — Update `markdown_links`

**Changes to `markdown_links::check`:**
```rust
// Before:
let resolved = source.skill_dir.join(&link.target);
if !resolved.exists() { ... }

// After:
if !ctx.skill_files[&source.name].contains(&link.target) { ... }
```

**Files touched:** `crates/skillet/src/lint/rules/markdown_links.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 5 — Update `oversized`

**Changes to `oversized::check_skill`:**
- Replace `read_compiled_tokens` (reads SKILL.md from disk) → `ctx.activation_tokens.get(&source.name)`
- When lockfile has `activation_tokens > 0`, use that (already the case). The fallback path that reads from disk uses `ctx.compiled_texts` instead.

**Changes to `oversized::check_fragments`:**
```rust
// Before:
pub fn check_fragments(config: &SkilletConfig, fragments_dir: &Path) -> Vec<Diagnostic>

// After:
pub fn check_fragments(config: &SkilletConfig, ctx: &LintContext) -> Vec<Diagnostic>
```
- Replace `read_dir` + `read_to_string` loop → iterate `ctx.fragment_tokens`

**Files touched:** `crates/skillet/src/lint/rules/oversized.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 6 — Update `unused_fragment`

**Changes:**
```rust
// Before:
pub fn check(source_files: &[SourceFile], fragments_dir: &Path, config: &SkilletConfig) -> Vec<Diagnostic>

// After:
pub fn check(source_files: &[SourceFile], ctx: &LintContext, config: &SkilletConfig) -> Vec<Diagnostic>
```
- Replace `fragments_dir.exists()` + `read_dir` → iterate `ctx.fragment_names`

**Files touched:** `crates/skillet/src/lint/rules/unused_fragment.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 7 — Update `duplication`

**Changes:**
```rust
// Before:
pub fn check(all_sources: &[SkillSource], lockfile: &Lockfile) -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>)

// After:
pub fn check(lockfile: &Lockfile, ctx: &LintContext) -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>)
```
- Replace `std::fs::read_to_string(SKILL.md)` per skill → `ctx.compiled_texts.get(name)`
- `all_sources` is no longer needed — skill names come from `ctx.compiled_texts.keys()`

**Files touched:** `crates/skillet/src/lint/rules/duplication.rs`, `crates/skillet-cli/src/lint.rs`

---

### Step 8 — Remove `workspace` I/O functions from library

After steps 1–7, the library's `workspace` module functions are only called by:
- The CLI's `build.rs`, `check.rs`, `lint.rs` (discovery + hashing)
- The library's `test_support.rs` (test-only)

**Move to CLI:**
- `discover_skills` → `skillet-cli/src/workspace.rs` (recreate it as the canonical I/O layer)
- `copy_dir_recursive` → same
- `hash_file` → same
- `load_fragment` → same
- `is_on_path` → same

**Keep in library (pure):**
- `SkillSource` struct (it's a data type, no I/O)

**Remove from library:**
- The `workspace` module's function bodies
- `walkdir` dependency

**Files touched:** `crates/skillet/src/workspace.rs`, `crates/skillet-cli/src/workspace.rs` (new), `crates/skillet/Cargo.toml`

---

### Step 9 — Remove `lockfile::read` / `lockfile::write` from library

These are convenience wrappers over `parse`/`serialize` + filesystem I/O. After step 8, only CLI and `test_support` call them.

**Move to CLI's workspace module.** Keep only `lockfile::parse` and `lockfile::serialize` in the library.

Update `test_support.rs` to use its own inline read/write.

**Files touched:** `crates/skillet/src/lockfile.rs`, `crates/skillet-cli/src/workspace.rs`, `crates/skillet/src/test_support.rs`

---

### Step 10 — Move `net/url_verify` to CLI

The URL verification module does TCP + TLS I/O. It's only consumed by CLI's `build.rs`.

**Move entire `net/` directory to CLI.** Remove `rustls` and `webpki-roots` from library deps.

**Files touched:** `crates/skillet/src/net/`, `crates/skillet-cli/src/net.rs`, `crates/skillet/Cargo.toml`

---

### Step 11 — Final cleanup

- Remove `walkdir`, `rustls`, `webpki-roots`, `sha2`, `hex` from library `Cargo.toml`
- Keep only: `toml`, `serde`, `anyhow`, `regex`, `chrono`, `gray_matter`, `tiktoken-rs`, `rayon`
- Remove `std::fs` usage from all library source (grep to confirm zero hits)
- Update library doc comment: "This crate provides pure domain logic — no I/O"

---

## Verification

After each step:
```bash
cargo test                           # all tests green
cargo build --benches -p skillet     # benchmarks compile
grep -rn "std::fs" crates/skillet/src/ | grep -v test  # decreasing toward zero
```

After step 11:
```bash
grep -rn "std::fs\|std::net\|walkdir\|TcpStream" crates/skillet/src/ | grep -v test
# Expected: zero results
```

---

## Constraints

- Each step is independently committable with all tests passing
- CLI output (format, exit codes, diagnostic text) must remain identical
- `--file` mode (editor integration) keeps working
- MinHash lockfile writeback keeps working
- `rayon` parallelism preserved (operates on in-memory data)
- No new allocations in hot paths beyond what's already there (pre-loading happens once at orchestration layer)
