# Story: Lint Skills for Quality

## As a
Skill author or CI pipeline

## I want to
Run `skillet lint` to detect problems across my skill workspace

## So that
I catch stale references, oversized skills, duplication, and structural issues before they reach consumers

## Acceptance Criteria

- [ ] `skillet lint` runs all enabled rules across the workspace
- [ ] `skillet lint <name>` runs rules for a single skill
- [ ] Rules and their severities:
  - `stale-path-ref` (error) — path ref doesn't resolve relative to skill dir
  - `stale-command-ref` (warning) — command not on `$PATH` (respects `allowed_commands`)
  - `stale-skill-ref` (error) — skill name doesn't exist in workspace
  - `invalid-frontmatter` (error) — missing `name`/`description`, or name ≠ directory name
  - `oversized-skill` (warning) — activation tokens exceed `max_activation_tokens`
  - `oversized-description` (warning) — discovery tokens exceed `max_discovery_tokens`
  - `oversized-fragment` (warning) — fragment tokens exceed `max_fragment_tokens`
  - `duplication` (warning) — near-verbatim content shared across skills
  - `stale-build` (error) — compiled SKILL.md doesn't match what build would produce
  - `unused-fragment` (warning) — fragment file not included by any skill
  - `untyped-backtick` (info) — backtick content matches ref pattern but lacks prefix
- [ ] Exit code: non-zero if any errors found, zero for warnings-only
- [ ] `--strict` promotes warnings to errors
- [ ] `--pedantic` shows info-level diagnostics
- [ ] Rules can be disabled via `disable = [...]` in `skillet.toml`
- [ ] Output is human-friendly colored text by default
- [ ] `--format json` produces machine-parseable output
