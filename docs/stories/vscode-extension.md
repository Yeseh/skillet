# VSCode Extension for Skillet

A VS Code extension providing syntax highlighting, linter diagnostics, and visual ref badges for `.pan` skill source files.

## Architecture

- **Linting**: Shell out to `skillet lint --format json` at workspace level on file save
- **Workspace detection**: Walk up from saved file to find `skillet.toml`; if not found, skip linting
- **Syntax highlighting**: Bundled TextMate grammar extending markdown
- **Typed ref styling**: Decoration overlays (pill/badge shape) on full backtick span
- **Output channel**: `Skillet` channel, written only on error
- **Settings**: `skillet.pedantic` (bool, off by default), `skillet.executablePath` (string, defaults to `"skillet"`)
- **Location**: `src/extensions/vscode/`
- **Publishing**: Local `.vsix` for v1

## User Stories

### US-1: Set up extension scaffold

**As a** developer
**I want** a working VS Code extension structure
**So that** I can build on it incrementally

**Acceptance criteria:**
- `src/extensions/vscode/` directory exists with `package.json`, `tsconfig.json`, `src/`, `README.md`
- `package.json` declares extension name, description, publisher, activation events for `*.pan` files
- TypeScript builds without errors
- `.vsix` can be packaged locally via `vsce package`

**Tasks:**
- Initialize extension with `yo code` or manual structure
- Add `npm ci && vsce package` to existing CI, depends on core `cargo build` and `cargo test`

---

### US-2: Provide TextMate grammar for `.pan` syntax highlighting

**As a** skill author
**I want** syntax highlighting for `.pan` files automatically
**So that** I don't have to manually configure `"*.pan": "markdown"` in `settings.json`

**Acceptance criteria:**
- `.pan` files show markdown syntax highlighting (headings, bold, links, code blocks, etc.)
- Frontmatter is recognized and styled as YAML
- Block includes (`{{> fragment-name}}`) are highlighted distinctively
- Typed ref prefixes (`ref::`, `cmd::`, `skill::`, `var::`, `env::`) are recognized as tokens
- Grammar extends built-in markdown grammar to avoid duplication

**Tasks:**
- Define TextMate grammar in `src/extensions/vscode/syntaxes/pan.tmLanguage.json`
- Add grammar contribution to `package.json`
- Test in VS Code that `.pan` files light up without manual configuration

---

### US-3: Detect workspace root and offer graceful fallback

**As a** skill author opening a `.pan` file
**I want** the extension to find my `skillet.toml` automatically
**So that** linting just works without configuration

**Acceptance criteria:**
- Extension walks up from saved file's directory until `skillet.toml` is found
- If `skillet.toml` is found, linting is enabled for that workspace
- If `skillet.toml` is not found, linting is skipped (no error, no warning)
- Multi-root workspaces are handled correctly (each file finds its own root)
- Works for skill files, fragment files, and reference/ files

**Tasks:**
- Implement walk-up logic in extension activation/save handler
- Test with nested and sibling workspace roots
- Document in extension README

---

### US-4: Run `skillet lint` on file save and display diagnostics

**As a** skill author
**I want** linting diagnostics to appear in the Problems panel
**So that** I see errors and warnings as I work

**Acceptance criteria:**
- On file save, invoke `skillet lint --format json` in the workspace root
- Parse JSON output and convert to VS Code diagnostics (error/warning/info)
- Diagnostics appear in Problems panel with rule name and message
- Diagnostics are cleared when the file is fixed
- No linting happens if `skillet` is not found (but see US-5 for the warning)

**Tasks:**
- Implement save event handler in extension
- Invoke `skillet lint --format json` as child process
- Parse JSON schema and map to `vscode.Diagnostic` objects
- Publish diagnostics to the active editor

---

### US-5: Warn user if `skillet` CLI is not found

**As a** skill author
**I want** a clear message if `skillet` is not installed
**So that** I'm not confused why no diagnostics appear

**Acceptance criteria:**
- On first save attempt, if `skillet` CLI is not found on `$PATH`, show a warning notification
- Notification includes "Don't show again" button
- Notification is shown once per session, not repeatedly
- Custom `skillet.executablePath` setting is respected when checking for the binary

**Tasks:**
- Detect non-zero exit when invoking `skillet` binary
- Show notification via `vscode.window.showWarningMessage`
- Persist "don't show again" state in memento or extension storage

---

### US-6: Decorate typed refs with badge-shaped overlays

**As a** skill author
**I want** typed refs to stand out visually as special constructs
**So that** I can instantly recognize them in the editor

**Acceptance criteria:**
- All typed ref backtick spans (`` `ref::./path` ``, `` `cmd::git` ``, etc.) are styled with a badge decoration
- Badge has a rounded pill shape with background color
- Badge covers the full backtick span
- Decoration is applied at the text editor level (not in the grammar)
- Same styling for skill files, fragment files, and reference/ files
- Decoration color is theme-aware and readable on light/dark backgrounds

**Tasks:**
- Define `TextEditorDecorationType` with `backgroundColor`, `borderRadius`, `padding`
- Scan for typed ref patterns (regex: `` `(ref|cmd|skill|var|env)::` ``)
- Apply decoration on document open and after each edit/save
- Test contrast on popular themes (One Dark, Dracula, Light themes)

---

### US-7: Expose `skillet.pedantic` setting for info-level diagnostics

**As a** a power user
**I want** to optionally see info-level linting rules
**So that** I can catch untyped backticks and other low-priority issues

**Acceptance criteria:**
- Extension setting `skillet.pedantic: boolean` exists, defaults to `false`
- When `true`, `skillet lint` is invoked with `--pedantic` flag
- Info-level diagnostics appear in the Problems panel
- Setting can be changed per-workspace or globally

**Tasks:**
- Add `skillet.pedantic` to `package.json` contribution point
- Read setting before invoking `skillet lint`
- Pass `--pedantic` flag conditionally
- Document in extension README

---

### US-8: Expose `skillet.executablePath` setting

**As a** a user with a non-standard `skillet` install location
**I want** to configure the path to the `skillet` binary
**So that** the extension finds it even if it's not on `$PATH`

**Acceptance criteria:**
- Extension setting `skillet.executablePath: string` exists, defaults to `"skillet"`
- When set, the extension invokes `skillet` at that path instead of relying on `$PATH`
- Relative paths are resolved relative to the workspace root
- Invalid/missing path is caught and reported in the output channel

**Tasks:**
- Add `skillet.executablePath` to `package.json` contribution point
- Read setting before invoking `skillet`
- Resolve relative paths against workspace root
- Document in extension README

---

### US-9: Log errors to a dedicated output channel

**As a** a user debugging extension issues
**I want** to see raw `skillet lint` output when something breaks
**So that** I can understand why diagnostics aren't appearing

**Acceptance criteria:**
- Extension creates a `Skillet` output channel
- Output channel is written to only on error (non-zero exit, JSON parse failure, stderr)
- Successful lint runs do not write to the channel (no noise)
- Channel is accessible via VS Code Output panel
- Error messages include the failed command and exit code

**Tasks:**
- Create output channel in extension activation
- Write stderr and exit code on lint failure
- Test with a broken `skillet` binary or malformed JSON

---

### US-10: Handle edge cases and invalid scenarios

**As a** an extension user
**I want** the extension to fail gracefully
**So that** it doesn't break my workflow

**Acceptance criteria:**
- If `skillet lint` produces invalid JSON, log error and don't crash
- If the workspace root cannot be determined, silently skip linting
- If the file is outside all workspace roots (multi-root), use the default workspace
- If `skillet.toml` is malformed, `skillet` itself will error; the extension reports it cleanly
- Concurrent saves don't spawn multiple `skillet` processes (debounce or queue)

**Tasks:**
- Implement JSON parse error handling
- Add debouncing/queuing for rapid saves
- Test with missing/malformed workspace config
- Test multi-root scenarios

---

### US-11: Document extension setup and usage

**As a** a contributor or end-user
**I want** clear documentation
**So that** I can install and use the extension

**Acceptance criteria:**
- `src/extensions/vscode/README.md` exists with installation, configuration, and troubleshooting sections
- Example `.vscode/settings.json` snippets are provided for common configurations
- Known limitations are documented (e.g., info diagnostics only with `--pedantic`)
- Architecture diagram shows how the extension invokes `skillet lint`

**Tasks:**
- Write README covering install, config, and examples
- Add troubleshooting section for common issues ("skillet not found", "no diagnostics appearing")
- Include example `settings.json`

---

### US-12: Integrate extension CI into main workflow

**As a** a maintainer
**I want** extension packaging to be verified in CI
**So that** broken `.vsix` builds don't go unnoticed

**Acceptance criteria:**
- CI job runs `npm ci && vsce package` in `src/extensions/vscode/`
- Job depends on core `cargo build` and `cargo test` completing successfully
- Job exits non-zero if packaging fails
- Packaged `.vsix` artifact is available for local testing

**Tasks:**
- Add CI step to existing GitHub Actions workflow
- Ensure step runs after core tests pass
- Document in CONTRIBUTING or CI config

---

## Out of Scope for v1

- Semantic token providers (theming API)
- Custom theme bundled with extension
- IntelliSense/autocomplete for fragment names or ref targets
- Marketplace publishing
- Watchers for `skillet.toml` or fragment changes (re-trigger lint on config change)
- Extension tests (manual testing only for v1)
