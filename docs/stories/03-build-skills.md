# Story: Build Skills from Source

## As a
Skill author who has written `.skill` source files with template directives

## I want to
Run `skillet build` to compile all sources into spec-compliant `SKILL.md` files

## So that
Agent runtimes can consume my skills without knowing skillet exists

## Acceptance Criteria

- [ ] `skillet build` finds all `.skill` files in the workspace skills directory
- [ ] Fragment includes (`{{> name }}`) are resolved and inlined (block-level)
- [ ] Explicit refs are compiled: `ref::`, `cmd::`, `skill::` have prefixes stripped (backticks preserved)
- [ ] `var::` refs are substituted with values from `[vars]` (no backticks in output)
- [ ] `env::` refs are substituted with declared default values from `[env]` (no backticks in output)
- [ ] Output `SKILL.md` is valid markdown with valid YAML frontmatter
- [ ] Frontmatter `name` field matches the skill directory name
- [ ] `skillet build <name>` compiles only the named skill
- [ ] Build fails (error) if: fragment not found, `var::` undefined, `env::` undeclared, `ref::` path missing, `skill::` not in workspace
- [ ] Build succeeds with warnings if: `cmd::` not on `$PATH`, URL unreachable (when enabled)
- [ ] `skillet.lock` is updated on successful build

## Notes

- Budget commands run against compiled output, not source
- Env var resolution uses declared defaults for reproducible builds
