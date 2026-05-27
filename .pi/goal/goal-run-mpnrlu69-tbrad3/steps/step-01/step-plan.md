# Step Plan

## Goal

Update lint reference extraction so supported references are recognized from the new parser while preserving the current lint-visible behavior.

## Summary

Switch `ParsedRefs::extract` from regex-driven typed/untyped scanning to `skillet-compiler`'s parser, while preserving markdown-link regex extraction and current lint-facing positions/classifications. Because the compiler parser is not yet consumable cross-crate and does not currently emit `var::`/`env::` refs, the generator should first expose and minimally extend that parser, then add focused regression tests around lint ref extraction.

## Constraints

- Keep the change surgical and centered on Phase 2 lint extraction; do not refactor unrelated compile/budget code or alter existing regex-based helpers that other subsystems still use (`typed_refs`, `extract_path_refs`, fragment expansion regexes).
- `ParsedRefs::extract` must use `PanSource::new` + `PanParse::new` + `parse()` for typed refs and untyped backticks; markdown links must continue to be extracted with the existing markdown-link regex because the parser does not handle markdown links.
- Preserve current lint-visible behavior for existing covered cases: same line/column locations, same trimmed ref values, same untyped classifications, and no new lint refs from unsupported parser kinds.
- Map parser ref kinds exactly for lint: `Reference` and `Path` -> `refs::RefKind::Ref`, `Skill` -> `Skill`, `Cmd` -> `Cmd`, `Var` -> `Var`, `Env` -> `Env`; skip `Agent` and `Url`.
- `RefSuspect` handling must slice the original source using `source_range`, strip the outer backticks, trim the inner content, and pass it through `classify_untyped`; unclassifiable suspects must still be ignored.
- Treat parser errors as best-effort parsing only: harvesting parser nodes must not introduce new diagnostics or suppress markdown-link extraction for otherwise readable files.
- If the generator touches `skillet-compiler`, only add the minimum surface needed for lint integration: public parser access and `var::`/`env::` token/node support with tests.

## Allowed Files

- crates/skillet/Cargo.toml
- crates/skillet/src/refs.rs
- crates/skillet/src/lint/pipeline.rs
- crates/skillet-compiler/src/lib.rs
- crates/skillet-compiler/src/lex.rs
- crates/skillet-compiler/src/parse.rs

## Implementation Steps

1. Add a path dependency from `crates/skillet` to `crates/skillet-compiler`, and make the parser consumable from outside that crate by exposing the parse module from `crates/skillet-compiler/src/lib.rs`.
2. Extend `crates/skillet-compiler/src/lex.rs` and `crates/skillet-compiler/src/parse.rs` just enough for lint needs: recognize `var::` and `env::` as typed ref kinds, emit matching `Node::Ref` variants, and add unit coverage proving those refs parse correctly without changing existing supported ref behavior.
3. Rewrite `ParsedRefs::extract` in `crates/skillet/src/refs.rs` to build a `PanSource`, run `PanParse`, and walk `parsed.nodes` once: convert supported `Node::Ref` entries into `TypedRef` values with trimmed `value`, `start`/`end` from `source_range`, and `line`/`col` from `PanSource::location_at`; convert supported `Node::RefSuspect` entries into `UntypedRef` via source slicing + `classify_untyped`; continue collecting markdown links with `extract_markdown_links`; ignore parser `Agent`/`Url` nodes and any unclassifiable suspects.
4. Keep Phase 2 contracts stable for lint consumers: only adjust `crates/skillet/src/lint/pipeline.rs` if needed for tests or imports, and do not change how `build_all_refs` flattens `ParsedRefs` into workspace refs.
5. Add focused regression tests in the touched files: parser tests for `var::`/`env::`; `ParsedRefs::extract` tests covering ref-kind mapping (including `path::` -> lint path ref and skipping `agent::`/`url::`), untyped backtick classification via `RefSuspect`, and coexistence with markdown links; update/add a pipeline-level test that proves Phase 2 still yields lint-consumable path refs from parsed output.

## Acceptance Criteria

- **[fmt]** (format, required): Rust formatting matches repository standards.
  - command: `cargo fmt --check`
- **[clippy]** (lint, required): Workspace clippy passes with warnings denied, including any new parser-facing code and tests.
  - command: `cargo clippy --workspace --all-targets -- -D warnings`
- **[build]** (test, required): The workspace still builds after wiring `skillet` to `skillet-compiler`.
  - command: `cargo build --locked`
- **[compiler-parser-tests]** (test, required): `skillet-compiler` unit tests pass, including the new `var::`/`env::` parser coverage.
  - command: `cargo test --locked -p skillet-compiler`
- **[parsed-refs-tests]** (test, required): Focused `ParsedRefs::extract` tests pass for parser mapping, skipped unsupported kinds, untyped suspect classification, and markdown-link coexistence.
  - command: `cargo test --locked -p skillet parsed_refs_extract_`
- **[phase2-pipeline-regression]** (test, required): Phase 2 extraction still produces lint-consumable typed/path refs from scanned sources.
  - command: `cargo test --locked -p skillet extract_refs_populates_typed_refs`
- **[lint-stale-ref-regressions]** (test, required): Existing lint regressions for stale path/skill/var/env refs still pass with parser-backed extraction.
  - command: `cargo test --locked -p skillet check_refs_`
- **[markdown-link-regressions]** (test, required): Markdown-link lint behavior remains unchanged because link extraction stays regex-based.
  - command: `cargo test --locked -p skillet markdown_links`
- **[workspace-tests]** (test, required): All existing tests across the workspace continue to pass.
  - command: `cargo test --locked`