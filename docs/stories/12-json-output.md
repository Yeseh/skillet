# Story: JSON Output for Tooling Integration

## As a
Developer integrating skillet into CI, editors, or dashboards

## I want to
Get structured JSON output from all skillet commands

## So that
I can parse results programmatically without scraping terminal output

## Acceptance Criteria

- [ ] All commands (`build`, `lint`, `budget`, `check`, `init`, `new`) support `--format json`
- [ ] Default output is human-friendly colored text
- [ ] JSON output includes all information shown in human output (no data loss)
- [ ] `lint` JSON includes: rule id, severity, message, file path, line number (where applicable)
- [ ] `budget` JSON includes: per-skill token counts (all three tiers), fragment breakdowns, totals
- [ ] `check` JSON includes: stale/fresh status per skill, which files differ
- [ ] `build` JSON includes: skills built, warnings encountered, lockfile path
- [ ] Exit codes are consistent regardless of output format
- [ ] JSON is printed to stdout; human diagnostic messages go to stderr (so JSON is always clean)
