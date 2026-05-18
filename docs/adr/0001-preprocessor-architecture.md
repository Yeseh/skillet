# ADR-0001: Preprocessor architecture with committed compiled output

## Status
Accepted

## Context
Skillet is a development kit for agent skills — part templating language, part linter, part quality toolkit. The Agent Skills spec defines SKILL.md as plain markdown with YAML frontmatter. We need to support fragments (shared reusable blocks), typed refs, and variable substitution without breaking the spec or requiring agent runtimes to understand skillet-specific syntax.

## Decision
Skillet uses a **preprocessor model**: authors write `.skill` source files containing template directives (`{{> fragment }}`, `ref::`, `var::`, `env::`). `skillet build` compiles these to spec-compliant `SKILL.md` files. Compiled output is committed to version control.

### Key properties:
- **Source files**: `{name}.skill` — contains template syntax
- **Compiled output**: `SKILL.md` — plain markdown, no skillet-specific syntax, consumable by any agent runtime
- **Freshness verification**: `skillet check` verifies committed output matches what build would produce (like `go generate` + `git diff --exit-code`)
- **Lockfile**: `skillet.lock` records hashes, token counts, and ref inventories for fast staleness checks and PR diffs

### Alternatives considered:
1. **Agent-runtime includes** — define an include format runtimes resolve natively. Rejected: breaks portability, requires runtime adoption.
2. **Source-only repo** — don't commit compiled output, consumers must install skillet. Rejected: breaks zero-dependency consumption of skills.
3. **In-markdown annotations only** — no build step, just lint existing SKILL.md files. Rejected: can't solve duplication (fragments require compilation).

## Consequences
- Authors must run `skillet build` (or CI must) after editing sources
- CI should run `skillet check` to catch uncommitted build drift
- The skill repo works without skillet installed (consumers just use SKILL.md)
- Fragment extraction eliminates cross-skill duplication at the source level
- Token budgets run against compiled output (accurate to what agents consume)
