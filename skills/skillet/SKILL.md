---
name: skillet
description: "Manage skills in a skillet workspace."
---

# skillet

Use this skill when working with AI agent skills in a project that uses `skillet` for skill management.

## Overview

Skillet is a CLI for authoring skills as `.pan` source files and compiling them to `SKILL.md` outputs that agent runtimes can consume directly.

A workspace contains:

- `skillet.toml` — workspace configuration
- `src/skills/<name>/<name>.pan` — skill source files
- `src/skills/_fragments/` — reusable shared fragments
- `skills/<name>/SKILL.md` — compiled outputs
- `skillet.lock` — lockfile with hashes and token counts

## Commands

| Command | Purpose |
|---|---|
| `skillet init` | Initialize a workspace |
| `skillet new <name>` | Scaffold a new skill source |
| `--format json`.
By default, output is plain text.

## Skill source format

A `.pan` file is Markdown with YAML frontmatter:

```markdown
---
name: my-skill
description: "One-line description for skill discovery."
---

# my-skill

Skill content here.
```

## Typed references

Inside `.pan` files you can use backtick typed refs to reference external resources:

- `ref::` + path — file reference (validated at build time)
- `cmd::` + command — shell command (checked on PATH)
- `skill::` + name — another skill in the workspace
- `var::` + name — variable from [vars]` in `skillet.toml`
- `env::` + name — environment variable with default from [env]`

## Fragments

Extract shared passages to `src/skills/_fragments/<name>.fragment.pan` and include with 