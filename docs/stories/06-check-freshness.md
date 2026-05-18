# Story: Check Build Freshness (CI)

## As a
CI pipeline or developer verifying before commit

## I want to
Run `skillet check` to verify that compiled `SKILL.md` files are up-to-date with their sources

## So that
I can catch uncommitted build drift and ensure the repo is always in a consistent state

## Acceptance Criteria

- [ ] `skillet check` compares current source hashes against `skillet.lock`
- [ ] If any `.skill` source has changed since last build, exits with code 1 and reports which skills are stale
- [ ] If compiled `SKILL.md` doesn't match what `skillet build` would produce, exits with code 1
- [ ] If lockfile is missing, exits with code 1 and suggests running `skillet build`
- [ ] If everything is fresh, exits with code 0 and reports success
- [ ] Fast path: uses source hashes from lockfile for comparison (doesn't need full recompilation)
- [ ] `--format json` produces machine-parseable output
