# Story: Initialize a Workspace

## As a
Skill author starting a new project

## I want to
Run `skillet init` to scaffold a workspace with sensible defaults

## So that
I have the correct directory structure and configuration to start authoring skills immediately

## Acceptance Criteria

- [ ] `skillet init` creates `skillet.toml` with default configuration
- [ ] `skillet init` creates `skills/` directory
- [ ] `skillet init` creates `skills/_fragments/` directory
- [ ] Generated `skillet.toml` contains all sections with sensible defaults (`[workspace]`, `[lint]`, `[build]`, `[vars]`, `[env]`)
- [ ] Running `skillet init` in a directory that already has `skillet.toml` produces an error (no overwrite)

---

# Story: Adopt Existing Skills

## As a
Skill author with existing `SKILL.md` files (e.g., migrating from hand-authored skills)

## I want to
Run `skillet init --adopt` to reverse-engineer my existing skills into `.skill` source files

## So that
I can start using skillet without rewriting my skills from scratch

## Acceptance Criteria

- [ ] `--adopt` detects existing `SKILL.md` files in the skills directory
- [ ] Each `SKILL.md` is copied as `{name}.skill` (content preserved as-is)
- [ ] Skill name is inferred from the directory name
- [ ] A `skillet.toml` is generated alongside the adopted sources
- [ ] Existing `SKILL.md` files are left in place (they become the initial compiled output)
