<div align="center">
    <p align="center">
      <img src="assets/logo.png" alt="Skillet" width="400" />
    </p>

  <p align="center">
    <strong>
        Toolkit for developing AI agents, skills, and prompts with Markdown
    </strong>
  </p>
</div>

---

# Introduction 

Skillet is a Rust CLI for authoring, compiling, linting, and budgeting AI agent skills.

Authors write skill source files in a lightweight `.pan` format, then compile them into plain `SKILL.md` files that an agent runtime can consume directly.

## Quick start

To install directly from GitHub:

```bash
cargo install --git https://github.com/Yeseh/skillet.git
skillet init
```

## What Skillet does

- initializes a skill workspace
- scaffolds new skill source files
- compiles `.pan` sources into `SKILL.md`
- expands shared fragments
- resolves typed refs like `ref::`, `cmd::`, `skill::`, `var::`, and `env::`
- writes a `skillet.lock` file with hashes, token counts, refs, and fragment usage
- checks whether generated output is stale
- lints skills for common quality issues
- reports discovery, activation, and transitive token budgets
- supports human and JSON output for every CLI command

## Workspace layout

A typical workspace looks like this:

```text
.
├── skillet.toml
├── skillet.lock
├── src/
│   └── skills/
│       ├── _fragments/
│       │   └── common.fragment.pan
│       └── my-skill/
│           └── my-skill.pan
└── skills/
    └── my-skill/
        └── SKILL.md
```

Source files live under `src/skills/`.

Compiled output is written to `skills/`.


## Core workflow

### 1. Initialize a workspace

```bash
skillet init
```

This creates:

- `skillet.toml`
- `src/skills/`
- `src/skills/_fragments/`
- `skills/`

To adopt existing compiled skills into source form:

```bash
skillet init --adopt
```

That copies existing `skills/<name>/SKILL.md` files into matching `src/skills/<name>/<name>.pan` files.

### 2. Create a new skill

```bash
skillet new my-skill
```

This creates `src/skills/my-skill/my-skill.pan` with starter frontmatter and a heading.

### 3. Build skills

```bash
skillet build
```

Or build a single skill:

```bash
skillet build my-skill
```

Build output:

- writes `skills/<name>/SKILL.md`
- updates `skillet.lock`
- validates typed refs during compilation
- optionally verifies URLs when enabled in config

Useful flags:

```bash
skillet build --offline
skillet build --strict
skillet build --format json
```

### 4. Check freshness

```bash
skillet check
```

This is intended as a CI-style freshness check. It exits non-zero when:

- a source file changed after the last build
- a compiled `SKILL.md` changed or is missing
- a fragment changed after the last build
- a skill exists on disk but not in the lockfile
- a lockfile skill no longer exists in source

### 5. Lint skills

```bash
skillet lint
```

Or lint a single skill:

```bash
skillet lint my-skill
```

Useful flags:

```bash
skillet lint --strict
skillet lint --pedantic
skillet lint --format json
```

The linter currently checks for things like:

- invalid or incomplete frontmatter
- stale path, skill, var, and env refs
- stale builds
- oversized skills, descriptions, or fragments
- unused fragments
- duplicated passages across skills
- untyped backticks that look like refs
- bad markdown links

### 6. Inspect token budgets

```bash
skillet budget
```

Or inspect one skill:

```bash
skillet budget my-skill
```

Budget reporting includes:

- **discovery**: name + description
- **activation**: full compiled `SKILL.md`
- **transitive**: activation plus files referenced through `ref::`

## Skill source format

A minimal skill source looks like this:

```markdown
---
name: my-skill
description: "Short description of the skill"
---

# My Skill

Use `cmd::git status` to inspect the repo.
Read `ref::./scripts/check.sh` before running.
See `skill::other-skill` for a related workflow.
Deploy to `var::project_name`.
CI default: `env::CI`.
```

### Fragments

Fragments live under `src/skills/_fragments/` and are included with:

```markdown
{{> common }}
```

A fragment file is named like:

```text
src/skills/_fragments/common.fragment.pan
```

Nested fragment includes are not supported.

### Typed refs

Skillet recognizes these typed refs in backticks:

- `ref::path/to/file`
- `cmd::some command --flag`
- `skill::other-skill`
- `var::project_name`
- `env::CI`

During build:

- `ref::`, `cmd::`, and `skill::` keep their backticks in compiled output, but lose the prefix
- `var::` and `env::` are substituted inline without backticks

## CLI summary

| Command | Purpose |
|---|---|
| `skillet init` | Initialize a workspace |
| `skillet init --adopt` | Adopt existing `SKILL.md` files as sources |
| `skillet new <name>` | Scaffold a new skill source |
| `skillet build [name]` | Compile one or all skills |
| `skillet check` | Verify generated output is up to date |
| `skillet lint [name]` | Run lint rules |
| `skillet budget [name]` | Show token budget information |

All commands support:

```bash
--format human
--format json
```

## Configuration

`skillet.toml` controls workspace paths, lint thresholds, tokenizer settings, and declared vars/env values.

The default workspace section looks like this:

```toml
[workspace]
skills_src_dir = "src/skills"
skills_out_dir = "skills"
fragments_dir = "src/skills/_fragments"
```

The generated config also includes default lint thresholds, tokenizer settings, a sample `project_name` var, and declared environment defaults like `CI` and `TEAM_NAME`.

## Lockfile

`skillet build` writes `skillet.lock` at the workspace root.

It records:

- source and compiled hashes
- discovery, activation, and transitive token counts
- fragment usage
- structured refs
- build metadata

## Editor setup

To get Markdown syntax highlighting for `.pan` files in VS Code, add this to your `settings.json`:

```json
"files.associations": {
    "*.pan": "markdown"
}
```

## Development

Run the test suite with:

```bash
cargo test
```

For local command help:

```bash
skillet --help
skillet build --help
skillet lint --help
```

## Project status

This repository is currently a small Rust CLI crate focused on the core workflow for skill authoring and validation. The design notes in `docs/initial-design.md` provide additional background on the project direction.
