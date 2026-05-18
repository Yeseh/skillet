# Story: Reference Detection and Validation

## As a
Skill author using paths, commands, skill names, and URLs in my instructions

## I want to
Have skillet automatically detect and validate these references

## So that
I'm alerted when references go stale instead of discovering broken links at runtime

## Acceptance Criteria

### Layer 1: Markdown Links
- [ ] Standard markdown links `[text](path)` are detected and file paths validated
- [ ] Standard markdown links `[text](url)` are detected; URLs validated only when `verify_urls = true`

### Layer 2: Explicit Annotations
- [ ] `ref::./path` — validated against filesystem relative to skill directory
- [ ] `cmd::command` — checked against `$PATH` (warning if missing, respects `allowed_commands`)
- [ ] `skill::name` — checked against workspace skill directories
- [ ] `var::name` — checked against `[vars]` in `skillet.toml`
- [ ] `env::name` — checked against `[env]` in `skillet.toml`

### Layer 3: Heuristic Inference
- [ ] Untyped backtick content is classified: path > URL > skill name > command > ignore
- [ ] Path heuristic: contains `/`, `./`, `../`, or ends with known extension
- [ ] URL heuristic: starts with `http://` or `https://`
- [ ] Skill heuristic: exact match against workspace skill directory names
- [ ] Command heuristic: first token is lowercase/hyphenated, has flag-like arguments
- [ ] Conservative approach: when in doubt, classify as ignore (no false positives)
- [ ] `untyped-backtick` info lint suggests adding explicit prefix

### General
- [ ] Detection order: Layer 1 → Layer 2 → Layer 3 (most specific first)
- [ ] All detected refs are recorded in `skillet.lock`
