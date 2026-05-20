# ADR-0004: lib crate is pure logic; CLI crate owns all I/O

## Status
Accepted

## Context

The `skillet` lib crate contained filesystem I/O throughout: `config::load` read `skillet.toml` from disk, `workspace::discover_skills` walked the directory tree, `init::run` and `new::run` created directories and wrote files, and command-entry functions like `build::run` and `lint::run` read `.pan` sources internally. This made the lib untestable without real disk state and prevented embedding the lib in non-CLI hosts (language servers, editor extensions, future programmatic consumers).

## Decision

The lib crate provides pure logic only. The CLI crate owns all filesystem and workspace integration.

**Boundary rule:** no `std::fs`, `std::io`, or path-walking code in lib. Lib functions receive pre-read data and return structured results. The CLI reads files, assembles inputs, calls lib, then writes outputs and formats results for the user.

**Key types:**
- `SourceUnit { name: String, content: String }` — a named pre-read source file, generic across skills, agent files, and any future compilable source type.
- `CompileContext { source: SourceUnit, fragments: HashMap<String, String>, config: BuildConfig }` — the full bundle the lib needs to compile one unit.
- Result types (e.g. `CompileResult`, `LintResult`) live in lib; the CLI formats them as text or JSON.

**Module reorganization:** lib modules are organized around domain concepts (`compile`, `lint`, `budget`, `lockfile`, `tokens`, `parse`, `refs`, `config`), not CLI commands. Command-mirrored modules (`init`, `new`, `check`) are removed from lib; their residual pure logic folds into the appropriate domain module.

**Config:** `SkilletConfig` and sub-types stay in lib. `config::load` (reads `skillet.toml` from disk) moves to CLI.

**Workspace discovery:** `workspace::discover_skills` and filesystem utilities move entirely to CLI. Lib has no `workspace` module.

**Tests:** lib tests are pure unit tests with no `tempfile` or disk state. Integration tests (full workspace → compiled output) live in `skillet-cli/tests/`.

## Alternatives considered

**Trait injection (filesystem trait):** lib defines a `FileSystem` trait; CLI injects a real implementation; lib tests use an in-memory fake. Rejected because it adds trait bounds to every lib function signature and keeps I/O concerns structurally present in lib, even if indirected.

**Command pattern (return operations):** lib returns a description of operations to perform; CLI executes them. Rejected as over-engineering for this codebase — the extra indirection buys nothing that approach A doesn't already give through structured result types.

## Consequences

- Lib has zero `std::fs` / `std::io` imports; can be embedded in any host without filesystem access.
- All lib functions are fast, pure unit-testable.
- CLI becomes the explicit orchestration layer: load config → discover sources → read content → call lib → write output.
- Future source types (agent files, prompt templates) fit naturally into `SourceUnit` without changing the lib API shape.
