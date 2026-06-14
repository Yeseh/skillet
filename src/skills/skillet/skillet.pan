---
name: skillet
description: "Manage skills in a skillet workspace."
---

# skillet

Use this skill when working with AI agent skills in a project that uses `skillet` for skill management.

## Overview

Skillet is a CLI for authoring skills as `.pan` source files and compiling them to `SKILL.md` outputs that agent runtimes can consume directly.

A workspace contains:

- ``skillet.toml`` — workspace configuration
- ``src/skills/<name>/<name>.pan`` — skill source files
- ``src/skills/_fragments/`` — reusable shared fragments
- ``skills/<name>/SKILL.md`` — compiled outputs
- ``skillet.lock`` — lockfile with hashes and token counts

## Commands

| Command | Purpose |
|---|---|
| ``skillet init`` | Initialize a workspace |
| ``skillet new <name>`` | Scaffold a new skill source |
| ``skillet build [name]`` | Compile one or all skills |
| ``skillet check`` | Verify compiled output is fresh |
| ``skillet lint [name]`` | Run quality lint rules |
| ``skillet budget [name]`` | Show token cost information |
| ``skillet publish`` | Publish plugin manifests to agent marketplaces |
| ``skillet skill list`` | List bundled skills |
| ``skillet skill print <name>`` | Print a bundled skill to stdout |

Commands that support formatting accept `--format json`.
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

- ``ref::`` + path — file reference (validated at build time)
- ``cmd::`` + command — shell command (checked on PATH)
- ``skill::`` + name — another skill in the workspace
- ``var::`` + name — variable from `[vars]` in `skillet.toml`
- ``env::`` + name — environment variable with default from `[env]`

## Fragments

Extract shared passages to ``src/skills/_fragments/<name>.fragment.pan`` and include one on its own line by wrapping the fragment name in the fragment delimiters — an opening `{` immediately followed by `>`, then the name, then the closing pair ``<}``. Includes are block-level and cannot be nested.

## Workflow

1. ``skillet new <name>`` — scaffold a skill
2. Edit ``src/skills/<name>/<name>.pan``
3. ``skillet build`` — compile to ``skills/<name>/SKILL.md``
4. ``skillet lint`` — check for quality issues
5. ``skillet check`` in CI — verify output is fresh
