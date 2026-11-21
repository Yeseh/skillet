---
status: accepted
---

Skillet workspaces need to support multiple independent source/output pairs so that skill sets can be versioned and published as separate plugins. We introduce a `[plugin.(name)]` section in `skillet.toml` with `src_dir`, `out_dir`, optional `fragments_dir`, and `version` fields. `[workspace]` holds only global config (fragments, lint, build, vars, env) — it never declares `src_dir` or `out_dir`. All skill sources are plugins. This is a breaking change from the prior single-workspace model.

## Considered Options

**Workspace as implicit default plugin** — `[workspace].src_dir`/`out_dir` kept for backwards compatibility, mutually exclusive with `[plugin.*]`. Rejected: backwards compatibility adds a two-model system with no benefit. A clean break keeps one mental model.

**Plugin output namespacing** — compiled skills land at `{out_dir}/{plugin-name}/{skill}/SKILL.md`. Rejected because it breaks the flat `skills/` convention expected by Claude Code and the `.skill` format.

## Consequences

Fragment scope follows a two-level pattern: `[workspace].fragments_dir` is global (available to all plugins); each `[plugin.*]` may declare its own `fragments_dir` for private fragments. A lint rule flags any plugin skill referencing a workspace-global fragment, since that fragment will not be available when the plugin is published standalone. Similarly, `skill::` refs are plugin-local — referencing a skill defined in a different plugin is a lint error, not a runtime concern.

`[lint]`, `[build]`, `[vars]`, and `[env]` remain workspace-level; plugins inherit them without override.

Build fails with a conflict error if two plugins would write a skill of the same name to the same `out_dir`. `skillet build`, `lint`, `check`, and `budget` all accept `--plugin <name>` to target a single plugin; default is all plugins.

`skillet.lock` remains a single workspace-wide file and is never distributed — it is a development artifact only.

## Amendment (2026-11-21)

Renamed `[module.*]` to `[plugin.*]` (and `--module` to `--plugin`, `cross-module-*` lint rules to `cross-plugin-*`) so the configuration matches agent-plugin terminology. No alias for the old name is kept.
