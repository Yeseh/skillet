# Reference

This document keeps the detailed workspace and file-format reference that does not need to live in the top-level README.

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

Source files live under `src/skills/`. Compiled output is written to `skills/`.

## Core workflow details

### Initialize a workspace

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

### Build output

```bash
skillet build
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

### Freshness checks

```bash
skillet check
```

This exits non-zero when:

- a source file changed after the last build
- a compiled `SKILL.md` changed or is missing
- a fragment changed after the last build
- a skill exists on disk but not in the lockfile
- a lockfile skill no longer exists in source

### Linting

```bash
skillet lint
skillet lint my-skill
```

Useful flags:

```bash
skillet lint --strict
skillet lint --pedantic
skillet lint --format json
```

The linter checks for things like:

- invalid or incomplete frontmatter
- stale path, skill, var, and env refs
- stale builds
- oversized skills, descriptions, or fragments
- unused fragments
- duplicated passages across skills
- untyped backticks that look like refs
- bad markdown links

### Token budgets

```bash
skillet budget
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

## Configuration

`skillet.toml` controls workspace paths, lint thresholds, tokenizer settings, and declared vars/env values.

### `[workspace]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `skills_src_dir` | string | `"src/skills"` | Directory where skill source `.pan` files are stored, relative to the project root. |
| `skills_out_dir` | string | `"skills"` | Directory where compiled `SKILL.md` outputs are written, relative to the project root. |
| `fragments_dir` | string | `"src/skills/_fragments"` | Directory where fragment `.fragment.pan` files are stored. |

### `[lint]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_activation_tokens` | integer | `4000` | Maximum token budget for a skill's activation section. Exceeding this triggers an oversized warning. |
| `max_discovery_tokens` | integer | `100` | Maximum token budget for a skill's discovery section (name + description). |
| `max_fragment_tokens` | integer | `500` | Maximum token budget for a single fragment file. |
| `allowed_commands` | list of strings | `["playwright", "docker", "kubectl"]` | Shell commands that skills are permitted to reference with `cmd::`. Commands not in this list are flagged. |
| `disable` | list of strings | `[]` | Rule IDs to silence (e.g. `"lint-missing-docs"`). |

### `[build]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tokenizer` | string | `"cl100k_base"` | Tokenizer model used for token counting. |
| `verify_urls` | boolean | `false` | When `true`, build verifies that URLs referenced in skills are reachable. Equivalent to passing `--strict` but persisted in config. |

### `[vars]`

A freeform key/value map of template variables available inside skill templates via `var::`. Values are plain strings substituted inline at build time.

```toml
[vars]
project_name = "my-project"
```

### `[env]`

Declares environment variables with fallback defaults. Each entry is a table with a `default` key. At build time, skillet reads the real environment variable; if it is unset, the `default` is used instead.

```toml
[env.CI]
default = "false"

[env.TEAM_NAME]
default = "engineering"
```

### Full example

```toml
[workspace]
skills_src_dir = "src/skills"
skills_out_dir = "skills"
fragments_dir = "src/skills/_fragments"

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

[env.CI]
default = "false"

[env.TEAM_NAME]
default = "engineering"
```

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

## Command help

```bash
skillet --help
skillet build --help
skillet lint --help
```
