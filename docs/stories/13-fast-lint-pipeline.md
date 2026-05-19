# 13 — Fast Lint Pipeline

Target: sub-second full workspace lint, ~5ms single-file lint for editor integration. No daemon.

## Data model

- [ ] Define `SourceFileType` enum: `Skill | ReferenceDocument`
- [ ] Define `SourceFile` struct: path, type, raw content, source hash (SHA256), token count, parsed frontmatter (Skill only), parse errors
- [ ] Define `Ref` enum with variants: `Skill | Cmd | PathRef | Var | Env | Untyped` — each carrying value, source file id, line, col
- [ ] Define `AllRefs` as `Vec<Ref>` collected across all files
- [ ] Extend `Lockfile` to store per-skill MinHash signatures (invalidated when compiled hash changes)

## Phase 1 — Parallel source scan

- [ ] Discover all `.pan` skill source files and `{skill}/reference/` documents in a single pass
- [ ] Read, hash (SHA256), and count tokens for every file in parallel via `rayon::par_iter`
- [ ] Parse YAML frontmatter for `Skill` files; collect parse failures as immediate diagnostics
- [ ] Produce `Vec<SourceFile>` as the phase output

## Phase 2 — Parallel ref extraction

- [ ] For each `SourceFile`, extract all typed refs (`ref::`, `cmd::`, `skill::`, `var::`, `env::`) and classify into `Ref` variants
- [ ] Extract markdown links; classify as `PathRef` (non-URL) or skip (URL links validated separately)
- [ ] Classify untyped backtick expressions using existing Layer 3 heuristic; emit `Ref::Untyped`
- [ ] Collect per-file `Vec<Ref>` into flat `AllRefs` via `rayon::par_iter`

## Phase 3 — Parallel rule execution

Run both branches concurrently with `rayon::join`.

### Per-skill rules (branch A)

- [ ] `invalid_frontmatter` — validate required fields from frontmatter parsed in Phase 1
- [ ] `stale_refs` — validate each `Ref::PathRef` exists on disk, `Ref::Skill` exists in SourceFile map, `Ref::Var`/`Ref::Env` declared in config
- [ ] `markdown_links` — validate non-URL path links resolve; flag bare URLs
- [ ] `untyped_backtick` — emit info diagnostic for each `Ref::Untyped`
- [ ] `oversized` — compare token count from Phase 1 against config thresholds (no re-tokenization)
- [ ] `stale_build` — compare source hash from Phase 1 against lockfile entry; skip SKILL.md read entirely if hash matches (short-circuit)

### Workspace rules (branch B)

- [ ] `unused_fragment` — collect fragment names referenced across `AllRefs`; compare against fragments dir
- [ ] `duplication` — rewrite using MinHash + LSH:
  - [ ] Read compiled `SKILL.md` files in parallel
  - [ ] Load cached MinHash signatures from lockfile when compiled hash is unchanged; recompute only on change
  - [ ] Build sentence windows and compute MinHash signatures for changed skills
  - [ ] Write updated signatures back to lockfile
  - [ ] Run LSH bucketing; compare only candidate pairs (not all pairs)
  - [ ] Emit `Warning` diagnostics for matches above overlap threshold

## Editor integration

- [ ] Add `skillet lint --file <path>` flag for single-file mode
- [ ] Single-file mode: Phase 1 for target file only + full skill discovery (readdir, no file reads) to populate SourceFile map for ref resolution
- [ ] Single-file mode: Phase 2 + Phase 3 branch A for target file only; skip all workspace rules
- [ ] Document VS Code extension wiring: shell out to `skillet lint --file ${file}` on save, parse output

## Plumbing

- [ ] Replace sequential `for source in &targets` loop in `lint::run` with phased barrier pipeline
- [ ] Thread `rayon` through all parallel phases; remove any remaining sequential skill loops
- [ ] Preserve `skill_name` single-skill filter for `skillet lint <skill>` (workspace rules still run)
- [ ] Add `--verbose` timing output: elapsed per phase, total
