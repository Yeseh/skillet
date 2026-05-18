# Skillet — Domain Glossary

## Terms

- **Skill**: A folder containing a `SKILL.md` file (compiled output) and optionally scripts, references, and assets. Follows the [Agent Skills](https://agentskills.io/home) open format.
- **Skill source**: A `{name}.pan` file — the authoring format that compiles to `SKILL.md`. Contains template directives (fragment includes, typed refs, vars).
- **Fragment**: A reusable block of skill instructions. File named `{name}.fragment.pan`, lives in the workspace-global fragments directory. Included via `{{> name }}` syntax. Block-level only, no parameters.
- **Ref**: A reference from a skill to an external entity. First-class concept with types: `ref::` (file path), `cmd::` (CLI command), `skill::` (another skill in workspace), `var::` (workspace variable), `env::` (declared environment variable).
- **Workspace**: A directory containing a `skillet.toml` and a skills directory. The unit of analysis for cross-skill operations (duplication detection, budget totals).
- **Budget**: The token cost of a skill measured in three tiers: discovery (description only, always loaded), activation (full SKILL.md), and transitive (activation + referenced files).
- **Discovery cost**: Tokens consumed by a skill's name + description at all times, even when the skill is not active.
- **Activation cost**: Tokens consumed when a skill is triggered — the full compiled SKILL.md.
- **Transitive cost**: Activation cost plus all files the skill instructs the agent to read.
- **Lockfile**: `skillet.lock` — committed workspace-level file recording source hashes, compiled hashes, token counts, fragment usage, and ref inventories per skill. Enables fast staleness checks and PR-diffable impact tracking.
