# Story: Lockfile Tracking

## As a
Skill author or reviewer looking at a PR

## I want to
See a diffable lockfile that shows exactly what changed (token counts, refs, fragment usage)

## So that
I can assess the impact of skill changes at a glance without running skillet locally

## Acceptance Criteria

- [ ] `skillet.lock` is generated/updated by `skillet build`
- [ ] Format is TOML (human-readable, diffable)
- [ ] Contains `[meta]` section: skillet version, build timestamp, tokenizer used
- [ ] Per-skill section contains: source hash, compiled hash, discovery/activation/transitive tokens, fragments used
- [ ] Per-skill `[refs]` subsection: lists all detected paths, commands, skills, URLs
- [ ] Per-fragment section: hash, token count, list of skills that use it
- [ ] `skillet check` uses lockfile hashes for fast staleness detection
- [ ] Lockfile is committed to version control
- [ ] Auto-generated header comment: "Do not edit. Run `skillet check` to verify freshness."
