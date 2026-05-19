<div align="center">
    <p align="center">
      <img src="assets/logo.png" alt="Skillet" width="400" />
    </p>

  <p align="center">
    <a href="https://github.com/Yeseh/skillet/actions/workflows/ci.yml">
      <img src="https://github.com/Yeseh/skillet/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI status" />
    </a>
  </p>

  <p align="center">
    <strong>
        Toolkit for developing AI agents, skills, and prompts with Markdown
    </strong>
  </p>
</div>

# Skillet

> [!WARNING]
> This project is experimental and subject to change.

Skillet is a Rust CLI for authoring skills in lightweight `.pan` files and compiling them into plain `SKILL.md` files that agent runtimes can consume directly.

## Quick start

```bash
cargo install --git https://github.com/Yeseh/skillet.git --package skillet-cli
skillet init
skillet new my-skill
skillet build
skillet lint
skillet budget
```

That gives you a workspace, scaffolds a skill, compiles it to `skills/<name>/SKILL.md`, checks it for common issues, and reports its token cost.

## Why use Skillet

- **Write once, ship plain Markdown.** Author in `.pan`, compile to runtime-friendly `SKILL.md`.
- **Catch problems early.** Validate typed refs, detect stale output, lint skills, and optionally verify URLs.
- **Track context cost.** Record discovery, activation, and transitive token counts in `skillet.lock` and inspect them with `skillet budget`.
- **Reduce repetition.** Detect duplicated passages across skills so shared guidance can move into fragments.
- **Integrate cleanly.** Commands that support formatting can emit JSON for tooling.

## Core commands

| Command | Purpose |
|---|---|
| `skillet init` | Initialize a workspace |
| `skillet new <name>` | Scaffold a new skill source |
| `skillet build [name]` | Compile one or all skills and update `skillet.lock` |
| `skillet check` | Verify generated output is fresh |
| `skillet lint [name]` | Run lint rules, including duplication checks |
| `skillet budget [name]` | Show token budget information |

Commands that support formatting accept `--format json`.
By default, output is plain text.

## Learn more

- **Reference:** [`docs/reference.md`](docs/reference.md)
- **Token budgets:** [`docs/stories/05-token-budget.md`](docs/stories/05-token-budget.md)
- **URL verification:** [`docs/stories/09-url-verification.md`](docs/stories/09-url-verification.md)
- **Duplication detection:** [`docs/stories/10-duplication-detection.md`](docs/stories/10-duplication-detection.md)
- **Lockfile details:** [`docs/stories/11-lockfile.md`](docs/stories/11-lockfile.md)
- **JSON output:** [`docs/stories/12-json-output.md`](docs/stories/12-json-output.md)
- **Design notes:** [`docs/initial-design.md`](docs/initial-design.md)

## Development

```bash
cargo test
```
