# Skillet — Initial Design

A development kit for agent skills. Part templating language, part linter, part toolkit for evaluating skill quality.

## Problem Statement

Developing serious agent skills surfaces recurring pain points:

1. **Stale references** — paths, commands, skill names, and URLs in skill instructions go out of date with no way to detect or validate this automatically.
2. **Opaque token costs** — the context impact of skills is not transparent; hard to evaluate whether a skill is worth its token budget.
3. **Instruction duplication** — reusing common instructions across skills leads to copy-paste that drifts over time.
4. **Unmeasurable impact** — the effect of enabling or disabling a skill is hard to quantify.

## Architecture

Skillet uses a **preprocessor model**. Authors write `.pan` source files containing template directives. `skillet build` compiles these into spec-compliant `SKILL.md` files that any agent runtime can consume without knowing skillet exists.

```
┌─────────────────┐       ┌──────────────┐       ┌────────────┐
│  .pan source    │──────▶│ skillet build │──────▶│  SKILL.md  │
│  (with directives)      │              │       │  (plain md) │
└─────────────────┘       └──────────────┘       └────────────┘
                                │
                                ▼
                          ┌──────────────┐
                          │ skillet.lock │
                          └──────────────┘
```

Compiled output is committed to version control. Consumers of the skill repo get working skills without needing skillet installed.

## File Layout

```
project/
├── skillet.toml              # Workspace configuration
├── skillet.lock              # Committed lockfile (hashes, tokens, refs)
└── skills/
    ├── _fragments/
    │   ├── check-adrs.fragment.pan
    │   └── common-tools.fragment.pan
    ├── diagnose/
    │   ├── diagnose.pan      # Source (authored)
    │   ├── SKILL.md          # Compiled output (committed)
    │   └── scripts/
    ├── caveman/
    │   ├── caveman.pan
    │   └── SKILL.md
    └── ...
```

## Source Format

Skill sources are markdown with YAML frontmatter and template directives:

```markdown
---
name: diagnose
description: Disciplined diagnosis loop for hard bugs and performance regressions.
---

# Diagnose

A discipline for hard bugs.

{{> check-adrs}}

Run `cmd::git bisect run` to find the offending commit.
See `skill::grill-with-docs` for domain validation.
Check `ref::./scripts/hitl-loop.template.sh` for the feedback loop template.

Deploy to `var::project_name` namespace.
CI status: `env::CI`.
```

### Compiled Output Behavior

| Source syntax | Compiled output |
|---------------|-----------------|
| `{{> fragment-name}}` | Fragment content inlined (block-level) |
| `` `ref::./path` `` | `` `./path` `` |
| `` `cmd::git bisect` `` | `` `git bisect` `` |
| `` `skill::diagnose` `` | `` `diagnose` `` |
| `` `var::project_name` `` | `my-project` (plain text, no backticks) |
| `` `env::CI` `` | `false` (plain text, resolved from declared default) |

## Fragments

Fragments are reusable blocks of skill instructions for eliminating cross-skill duplication.

- File naming: `{name}.fragment.pan`
- Location: workspace-global `_fragments/` directory only
- Include syntax: `{{> name }}` (block-level only, must be on its own line)
- No parameters (v1)

## Reference Detection

Three layers, evaluated in order of specificity:

### Layer 1: Markdown Links

Standard markdown links (`[text](url)` and `[text](path)`) are parsed structurally from the AST.

- File paths validated against filesystem
- URLs optionally verified via HTTP HEAD (opt-in)

### Layer 2: Explicit Annotations

Typed refs in backticks with a prefix:

| Prefix | Type | Validation |
|--------|------|------------|
| `ref::` | File path | Must exist relative to skill directory |
| `cmd::` | CLI command | Checked against `$PATH` (warning if missing) |
| `skill::` | Workspace skill | Must exist as a skill directory |
| `var::` | Variable | Must be declared in `skillet.toml` `[vars]` |
| `env::` | Environment variable | Must be declared in `skillet.toml` `[env]` |

### Layer 3: Heuristic Inference

Untyped backtick content is classified by pattern matching (conservative, low false-positive):

- **Path**: contains `/`, `./`, `../`, or ends with known extension
- **URL**: starts with `http://` or `https://`
- **Skill name**: exact match against workspace skill directory names
- **Command**: first token is lowercase/hyphenated with flag-like arguments
- **Ignore**: everything else

An `untyped-backtick` info-level lint nudges authors toward explicit annotations.

## Token Budget

Skills are measured in three tiers:

| Tier | What's measured | When loaded |
|------|-----------------|-------------|
| **Discovery** | `name` + `description` from frontmatter | Always (all skills) |
| **Activation** | Full compiled SKILL.md | When skill is triggered |
| **Transitive** | Activation + files the skill instructs the agent to read | When skill is triggered |

`skillet budget` output:

```
Skill            Discovery   Activation   Transitive   Fragments
──────────────────────────────────────────────────────────────────
caveman              42 tk      520 tk        520 tk   (none)
diagnose             58 tk    1,890 tk      2,340 tk   check-adrs (120 tk)
grill-with-docs      61 tk      980 tk      1,620 tk   check-adrs (120 tk)
──────────────────────────────────────────────────────────────────
TOTAL (discovery)   161 tk
TOTAL (all active)                          4,480 tk
```

Tokenizer: `cl100k_base` (configurable).

## Lint Rules

| Rule | Severity | Trigger |
|------|----------|---------|
| `stale-path-ref` | error | Path ref doesn't resolve |
| `stale-command-ref` | warning | Command not on `$PATH` |
| `stale-skill-ref` | error | Skill name doesn't exist in workspace |
| `invalid-frontmatter` | error | Missing `name`/`description`, or name ≠ directory name |
| `oversized-skill` | warning | Activation tokens exceed threshold |
| `oversized-description` | warning | Discovery tokens exceed threshold |
| `oversized-fragment` | warning | Fragment tokens exceed threshold |
| `duplication` | warning | Near-verbatim shared content across skills |
| `stale-build` | error | Compiled SKILL.md doesn't match build output |
| `unused-fragment` | warning | Fragment file not included by any skill |
| `untyped-backtick` | info | Backtick content matches a ref pattern but lacks explicit prefix |

Severity behavior:
- **error**: `skillet build` fails, `skillet lint` exits non-zero
- **warning**: `skillet build` succeeds, `skillet lint` reports but exits zero
- **info**: only shown with `--pedantic` or explicit opt-in
- `--strict` promotes warnings to errors (for CI)

## Configuration

`skillet.toml` at workspace root:

```toml
[workspace]
skills_dir = "skills"
fragments_dir = "skills/_fragments"

[lint]
max_activation_tokens = 4000
max_discovery_tokens = 100
max_fragment_tokens = 500
allowed_commands = ["playwright", "docker", "kubectl"]
disable = []

[build]
tokenizer = "cl100k_base"
verify_urls = false

[vars]
project_name = "my-project"

[env]
CI = { default = "false" }
TEAM_NAME = { default = "engineering" }
```

Environment variable access: only declared vars are accessed. The full environment is never queried.

## Lockfile

`skillet.lock` is workspace-level, committed, and auto-generated by `skillet build`:

```toml
# Auto-generated by skillet build. Do not edit.

[meta]
skillet_version = "0.1.0"
built_at = "2026-05-18T14:30:00Z"
tokenizer = "cl100k_base"

[skills.diagnose]
source_hash = "sha256:abc123..."
compiled_hash = "sha256:def456..."
discovery_tokens = 58
activation_tokens = 1890
transitive_tokens = 2340
fragments_used = ["check-adrs"]

[skills.diagnose.refs]
paths = ["./scripts/hitl-loop.template.sh"]
commands = ["git", "playwright"]
skills = []
urls = []

[fragments.check-adrs]
hash = "sha256:..."
tokens = 120
used_by = ["diagnose", "grill-with-docs"]
```

## CLI Commands

```
skillet init              # Scaffold workspace (skillet.toml, skills/, _fragments/)
skillet init --adopt      # Adopt existing SKILL.md files into .pan sources
skillet new <name>        # Scaffold a new skill (minimal: frontmatter + heading)

skillet build             # Compile all .pan sources → SKILL.md + update lockfile
skillet build <name>      # Compile a single skill

skillet lint              # Run all lint rules across workspace
skillet lint <name>       # Lint a single skill

skillet budget            # Show token cost table for all skills
skillet budget <name>     # Show breakdown for one skill (with fragment contributions)

skillet check             # Verify compiled output is up-to-date (CI command, exit 1 if stale)
```

All commands support `--format json` for machine-parseable output.

## URL Verification

Opt-in via `verify_urls = true` in config. Implementation is **isolated in a dedicated module** (`net/url_verify.rs`) for auditability — all network I/O lives there and nowhere else.

### Security controls:
1. Strict URL parser — reject non-http(s), reject ambiguous IP representations
2. DNS resolve first, check resolved IP against blocklist before connecting
3. Blocklist: RFC 1918, link-local, loopback (IPv4 and IPv6, including hex/octal/mapped)
4. **No redirects followed** — any redirect response (3xx) means the URL exists and is reachable; the user can follow the redirect themselves
5. HEAD request only — read status line only (first line, max 128 bytes), ignore all response headers, close immediately
6. 5s hard wall-clock timeout
7. Concurrency cap: 5 simultaneous checks
8. TLS cert verification enforced (no disable option)
9. Minimal headers, no cookies, no auth
10. Per-build result cache
11. `--offline` flag disables all checks regardless of config

### Result classification:
- DNS failure / connection refused / timeout → `unreachable` (warning)
- 2xx, 3xx → `ok` (3xx = path exists, redirect is the site's concern)
- 401, 403 → `ok` (exists, auth-gated)
- 404, 410 → `broken` (warning)
- 5xx → `possibly-down` (info)

## Duplication Detection

Near-verbatim detection for v1:
- Normalize whitespace and case
- Find shared n-grams (3+ sentence sequences with >80% overlap)
- Flag as warning with suggestion to extract into a fragment

Semantic duplication (embedding-based) is a future direction.

## Crate Architecture

```
src/
├── main.rs              # CLI entry (clap)
├── config.rs            # skillet.toml parsing
├── workspace.rs         # Skill discovery, workspace resolution
├── parse.rs             # .pan source parsing (frontmatter + markdown + refs)
├── refs/
│   ├── mod.rs           # Ref types, classification
│   ├── heuristic.rs     # Layer 3 inference
│   └── annotated.rs     # Layer 2 explicit ref:: parsing
├── fragments.rs         # Fragment resolution, inclusion
├── build.rs             # Compilation pipeline (.pan → SKILL.md)
├── lint/
│   ├── mod.rs           # Lint engine, rule registry
│   ├── rules/           # One file per rule
│   └── severity.rs      # Error/warning/info handling
├── budget.rs            # Tokenization, cost calculation
├── lockfile.rs          # Lock generation and comparison
├── net/
│   └── url_verify.rs    # ISOLATED: URL reachability (audit boundary)
└── cli/
    ├── build.rs
    ├── lint.rs
    ├── budget.rs
    ├── check.rs
    ├── init.rs
    └── new.rs
```

## Dependencies

| Purpose | Crate |
|---------|-------|
| CLI | `clap` (derive) |
| Config | `toml` + `serde` |
| Markdown parsing | `pulldown-cmark` |
| YAML frontmatter | `serde_yaml` |
| Tokenization | `tiktoken-rs` |
| HTTP (url verify) | `rustls` + `std::net::TcpStream` (raw, no HTTP library) |
| URL parsing | `url` |
| DNS resolution | `hickory-resolver` (or std) |
| Colored output | `owo-colors` |
| Filesystem | `walkdir` |
| Diffing | `similar` |

No async runtime. Blocking I/O is sufficient for a dev tool with bounded concurrency.

## Error Handling

| Failure | Severity | Build behavior |
|---------|----------|----------------|
| `ref::` path doesn't exist | error | Fails |
| `cmd::` not on $PATH | warning | Succeeds |
| `skill::` not in workspace | error | Fails |
| `var::` not in `[vars]` | error | Fails |
| `env::` not in `[env]` | error | Fails |
| URL unreachable (when enabled) | warning | Succeeds |
| Fragment not found | error | Fails |

`--strict` promotes all warnings to errors for CI use.

## Future Directions (out of scope for v1)

- **Behavioral impact evaluation** — run test prompts with/without a skill, diff agent behavior
- **Conflict detection** — identify skills that contradict each other
- **Semantic duplication** — embedding-based similarity detection
- **Fragment parameters** — `{{> name key="value" }}` for parameterized reuse
- **IDE integration** — LSP for `.pan` files (diagnostics, completion, go-to-fragment)
