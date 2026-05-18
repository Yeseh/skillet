# Story: View Token Budget

## As a
Skill author evaluating the context cost of my skills

## I want to
Run `skillet budget` to see token costs for all skills in my workspace

## So that
I can make informed decisions about which skills are worth their context impact

## Acceptance Criteria

- [ ] `skillet budget` displays a table with columns: Skill, Discovery, Activation, Transitive, Fragments
- [ ] Discovery = tokens for `name` + `description` only
- [ ] Activation = tokens for full compiled `SKILL.md`
- [ ] Transitive = activation + tokens for files the skill instructs the agent to read
- [ ] Fragments column shows which fragments are included and their individual token contribution
- [ ] Table includes workspace totals (total discovery, total all-active)
- [ ] `skillet budget <name>` shows detailed breakdown for a single skill
- [ ] Tokenizer is configurable via `skillet.toml` (`cl100k_base` default)
- [ ] Token counts match what's recorded in `skillet.lock`
- [ ] `--format json` produces machine-parseable output
