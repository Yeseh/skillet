# ADR-0002: Three-layer ref detection with conservative heuristics

## Status
Accepted

## Context
Skills reference external entities (file paths, CLI commands, other skills, URLs) that can go stale. We need to detect and validate these references. Authors range from "just writes markdown" to "wants precise control" — the detection strategy must serve both.

## Decision
Ref detection uses three layers, evaluated in order of specificity:

### Layer 1: Markdown link detection
Standard markdown links (`[text](url)` and `[text](path)`) are parsed structurally. URLs are optionally verified via HTTP HEAD (opt-in via config). File paths are validated against the filesystem.

### Layer 2: Explicitly annotated refs in backticks
Authors can annotate refs with a type prefix:
- `` `ref::./scripts/foo.sh` `` — file path
- `` `cmd::git bisect run` `` — CLI command
- `` `skill::diagnose` `` — workspace skill
- `` `var::project_name` `` — workspace variable (from `[vars]`)
- `` `env::CI` `` — declared environment variable (from `[env]`)

These are unambiguous and always validated.

### Layer 3: Heuristic inference on untyped backtick content
Content in single backticks without a prefix is classified by pattern matching:
- **Path**: contains `/`, `./`, `../`, or ends with known extension
- **URL**: starts with `http://` or `https://`
- **Skill name**: exact match against workspace skill directory names
- **Command**: first token is lowercase/hyphenated + has flag-like arguments
- **Ignore**: everything else

Conservative — when in doubt, don't flag. `untyped-backtick` info-level lint suggests adding explicit prefixes.

### Compiled output behavior:
- `ref::`, `cmd::`, `skill::` → prefix stripped, backticks preserved
- `var::`, `env::` → fully substituted, no backticks (plain text value)

## Consequences
- Zero adoption cost for existing skills (layer 3 works immediately)
- Precision improves as authors adopt explicit annotations (layer 2)
- False positives are minimized (conservative heuristics + suppressions in skillet.toml)
- `var::` and `env::` enable DRY without fragments (single-value substitution)
