# Skillet VS Code Extension

Syntax highlighting, linting diagnostics, and visual ref badges for `.pan` skill source files.

## Features

- **Syntax highlighting** — `.pan` files receive markdown syntax highlighting with:
  - YAML frontmatter styled as YAML
  - Block includes (`{{> fragment-name}}`) highlighted distinctively
  - Typed ref prefixes (`ref::`, `cmd::`, `skill::`, `var::`, `env::`) recognised as tokens
- **Linting** — on file save, runs `skillet lint --format json` and shows diagnostics in the Problems panel
- **Ref badge decorations** — all typed ref backtick spans are styled with a rounded pill badge
- **Output channel** — `Skillet` output channel captures errors and debug info (only written on failure)

## Requirements

- `skillet` CLI installed and available on `$PATH` (or configured via `skillet.executablePath`)
- VS Code 1.85 or later

## Installation

Install from a `.vsix` package:

```sh
# From the extension directory
npm ci
npx vsce package
code --install-extension skillet-0.1.0.vsix
```

## Configuration

| Setting | Type | Default | Description |
|---|---|---|---|
| `skillet.executablePath` | `string` | `"skillet"` | Path to the `skillet` binary. Relative paths are resolved against the workspace root. |
| `skillet.pedantic` | `boolean` | `false` | Pass `--pedantic` to show info-level diagnostics (untyped backticks, etc.). |

### Example `.vscode/settings.json`

```json
{
  // Use a specific skillet binary
  "skillet.executablePath": "/usr/local/bin/skillet",

  // Enable pedantic mode for info-level diagnostics
  "skillet.pedantic": true
}
```

Per-workspace override (in `.vscode/settings.json`):

```json
{
  "skillet.pedantic": false
}
```

## How It Works

```
.pan file saved
     │
     ▼
Walk up from file to find skillet.toml
     │ not found → silently skip
     │ found
     ▼
Spawn: skillet lint --format json [--pedantic]
     │ ENOENT → warn once "skillet not found"
     │ stderr  → write to Skillet output channel
     │ invalid JSON → write to Skillet output channel
     ▼
Parse JSON array of diagnostics
     ▼
Publish to VS Code Problems panel
```

## Troubleshooting

**No diagnostics appear**

1. Check that `skillet` is on your `$PATH`: `which skillet` / `skillet --version`
2. Check that a `skillet.toml` exists in a parent directory of your `.pan` file
3. Open the **Skillet** output channel (`View → Output → Skillet`) for error details
4. Try setting `skillet.executablePath` to an absolute path

**"skillet not found" warning**

Install the `skillet` CLI: see the [skillet README](../../../README.md) for instructions. Alternatively, set `skillet.executablePath` to the full path of your `skillet` binary.

**Info diagnostics not showing**

Enable `skillet.pedantic: true` in your settings — info-level rules are hidden by default.

## Known Limitations

- No IntelliSense/autocomplete for fragment names or ref targets (v1)
- No watchers for `skillet.toml` or fragment changes; save a `.pan` file to re-trigger lint
- Marketplace publishing not yet configured; install from `.vsix`
- Extension tests are manual only (v1)
