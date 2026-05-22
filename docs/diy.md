---
created: 2026-05-21
tags: [skillet, tutorial, rust, learning, parsers, compilers]
related:
  - "[[Skillet]]"
  - "[[Right-Sized Architecture]]"
  - "[[Rust Linters and Formatters - Skillet Design Notes]]"
---

# Skillet — Build It Yourself

A staged, hands-on guide to constructing the architecture sketched in [[Right-Sized Architecture]]. The goal is *learning*, not productivity — each stage gives you objectives, design questions, gotchas, and reading, but no full code. You write the code; this note is your compass.

The stages are sequential. Each one produces something runnable end-to-end before you move on. Resist the temptation to scaffold everything up front — you'll learn more by feeling the rough edges of each stage before deciding what the next abstraction needs to do.

A few rules for yourself before you start:

1. **No copy-pasted parser tutorials.** Read other parsers' code (links below), then close the tab and write yours.
2. **Each stage ends with `cargo run` against a real `.pan` file.** If it doesn't run, you're not done.
3. **Write the test before the refactor, not before the feature.** Match [[Jesse's Code Opinions]] — solve the problem first, then make it testable.
4. **Commit at every stage boundary.** Future-you will want the diff.

---

## Stage 0 — Workspace setup

**Objective:** A four-crate Cargo workspace that builds cleanly and runs a trivial CLI.

**What you'll create:**

```
skillet/
  Cargo.toml                    # [workspace] members = [...]
  crates/
    skillet-ast/                # lib
    skillet-parser/             # lib, depends on skillet-ast
    skillet-compiler/           # lib, depends on skillet-ast
    skillet/                    # bin, depends on all three
```

**Design questions:**

- Should `skillet-compiler` depend on `skillet-parser`, or should the binary be the only thing that wires them together? (Hint: think about who you'd want to be able to use the compiler without paying for the parser.)
- What goes in the workspace `Cargo.toml` versus each crate's? (Common deps via `[workspace.dependencies]` vs. per-crate.)

**Acceptance:**

- `cargo build --workspace` succeeds with zero warnings.
- `cargo run --bin skillet -- --version` prints something.
- Your `clippy::pedantic` baseline is clean (turn it on now; it's painful to retrofit).

**Reading:** the top-level `Cargo.toml` of `astral-sh/ruff` is a good model — note how they share lints, profile, and dependency versions across the workspace.

---

## Stage 1 — Source model

**Objective:** A type that owns a `.pan` file's bytes and lets the rest of the system borrow from it.

**What to figure out:**

- What's the right ownership shape? `String`? `Arc<str>`? `Box<str>`? Why does each one fail or succeed?
- How will downstream code refer to a slice of the source — `&'src str`? `Range<u32>`? Both? When does each work?
- How do you turn a byte offset back into a `(line, column)` for diagnostics? (Don't store line/col on every token — compute it lazily from a side-table.)

**Gotchas:**

- `String::as_str()` returns a slice tied to the `String`'s borrow — if your AST holds `&str` slices and the `String` moves, you have a problem. The crate-level lifetime story for this is the first thing that will bite you. Decide early.
- UTF-8 vs chars. Markdown is mostly ASCII-structural but bodies are arbitrary UTF-8. What's your offset unit?

**Acceptance:**

- A `PanSource` (or whatever you name it) that can be constructed from a file path, exposes the bytes, and can answer "what line/column is byte offset N?" with a single allocation.

**Reading:** `ruff_text_size` (the crate, ~200 lines) shows how the rust-analyzer ecosystem does this. Read it, then write your own simpler version.

---

## Stage 2 — Tokens and the lexer

**Objective:** A hand-written lexer that turns bytes into a stream of tokens with positions.

**What to figure out:**

- What *are* the tokens of `.pan`? Markdown is structural — what counts as a token vs. what counts as opaque body text?
- How do you handle the special constructs (`{{> name }}`, `var::FOO`, `ref::path`, `cmd::name`, `skill::name`)? Are they whole tokens, or are they multi-token sequences (e.g., `{{>` + name + `}}`)?
- Frontmatter — is it a token, or is it parsed separately by a YAML/TOML library before the lexer even runs?
- Code fences — when the lexer sees ```` ``` ```` what state does it enter? How does it know when to exit?

**Design questions:**

- Streaming iterator vs. `Vec<Token>`? At skillet's scale, materializing the whole vector is cheap and makes the parser dramatically easier. Lean that way.
- Are you returning errors from the lexer, or producing error tokens that the parser handles? (Hint: error tokens give better recovery; errors abort the stream.)

**Gotchas:**

- The naive "is this character alphanumeric?" check via `char::is_alphanumeric` does a UTF-8 lookup. For ASCII-only sub-grammars (identifiers, keywords), a 256-byte lookup table is faster *and* makes the code intent clearer.
- Don't allocate per token. Tokens carry `&str` slices into the source or `Range<u32>` byte offsets. No `String`.
- Be careful with `{{` and `{{>` — your lexer needs lookahead. Decide how much (one byte? two?).

**Acceptance:**

- Given a `.pan` file, `lex(&source) -> Vec<Token>` produces a stream you can `println!("{:?}", ...)` and visually verify.
- A handful of unit tests (golden token streams for representative inputs).

**Reading:**
- The lexer in `ruff_python_parser/src/lexer.rs` is large but the structural pattern (a struct with `source`, `cursor`, `state` and a method per token kind) is the model.
- `pulldown-cmark`'s `firstpass.rs` shows how a Markdown-specific lexer handles block-level structure — useful even though you're not using it.

---

## Stage 3 — AST and parser

**Objective:** A typed AST and a recursive-descent parser that produces it from the token stream.

**What to figure out:**

- What's the shape of your AST? See [[Right-Sized Architecture]] for the proposed `Block` enum; you can deviate but justify it. Why an enum-of-variants instead of a trait object hierarchy?
- How do you store a node's source range? On every node? On a side-table? (Hint: on every node, but consistently — pick one location and stick to it.)
- How does your parser handle errors? Does it abort on the first one, or does it recover and continue (collecting multiple)?

**Design questions:**

- Should fragments (`{{> name }}`) be parsed as a single `Fragment` block, or as a `Markdown` block containing an interpolated expression? The answer drives whether substitution happens during parse or during compile.
- Frontmatter — own AST type or just `serde_yaml::Value`? What's the tradeoff?
- Code fences — opaque body string, or do you also tokenize what's inside? (Hint: the body is opaque to skillet; treat it as a single string.)

**Gotchas:**

- Pratt parsing isn't needed at this scale — `.pan` has no operator precedence. A flat block-by-block parser is fine.
- If you store `&'src str` in the AST, your AST is generic over a source lifetime. That's a choice with ergonomic consequences. The alternative is to store `Range<u32>` and look up the text via the source when needed. Both are defensible; pick one and document why.

**Acceptance:**

- `parse(&source) -> Result<PanFile, Vec<ParseError>>` works on at least three real `.pan` files from the existing skillet repo.
- Pretty-printing the AST (a quick `Debug` walk) shows every construct you care about as a typed node, not as a generic "text with substring markers".
- Multiple parse errors come back in a single run when present.

**Reading:**
- matklad, [Resilient LL Parsing Tutorial](https://matklad.github.io/2023/05/21/resilient-ll-parsing-tutorial.html) — read this *before* you start. It will save you a week.
- matklad, [Simple but Powerful Pratt Parsing](https://matklad.github.io/2020/04/13/simple-but-powerful-pratt-parsing.html) — skim. You don't need Pratt today but knowing when you would is useful.

---

## Stage 4 — The compiler as a single visitor

**Objective:** A function that walks the AST once and produces the `SKILL.md` output plus diagnostics plus a token count.

**What to figure out:**

- What's the right return type? A struct with `output`, `diagnostics`, `tokens`, `refs_used`? Or do you accept an output `String` by `&mut`? Both work; what are the testability implications?
- How is the `CompileContext` (resolved fragments, vars, the skill registry) constructed? When does it get built — once per project, once per file?
- Where does fragment expansion happen — inline as you walk, or as a transformation pass that produces an "expanded AST"? Pros and cons of each.

**Design questions:**

- The visitor pattern in Rust has two common shapes: a trait with methods you override (`Visitor::visit_block(&mut self, b: &Block)`), or a plain function with a `match`. Which fits skillet's scale? Why?
- Token counting: do you count as you emit, or do you count the final `SKILL.md` string in one pass at the end? (Hint: counting as you emit is free because you're already touching every byte. End-pass counting is simpler to test in isolation.)

**Gotchas:**

- Fragment expansion needs a guard. Not graph cycle detection — just a `HashSet<&str>` of "fragments currently being expanded in this chain" passed down the recursion. Five lines. Forgetting this turns `{{> a }} → {{> b }} → {{> a }}` into a stack overflow.
- Don't reach for `Box<dyn Rule>`. Plain `fn validate_ref(...)` functions called from the `match`. You have under ten rules total.
- Resist deferring work to a "second pass" until you have a rule that actually requires it. Cross-file checks (does this `skill::foo` exist?) are one obvious case — they need every file parsed first. That's a function called after the per-file loop, not a `Deferred` queue.

**Acceptance:**

- Running `skillet build` on an existing skill produces a `SKILL.md` byte-for-byte identical to whatever skillet produces today.
- Introducing an unknown `ref::` produces a diagnostic with the correct line and column.
- Introducing a fragment cycle produces a diagnostic, not a stack overflow.

**Reading:**
- `crates/ruff_linter/src/checkers/ast/mod.rs` in ruff — the doc-comment at the top is the canonical statement of the "single-pass visitor with phases" design. The implementation is more than skillet needs; just read the design comment.

---

## Stage 5 — Diagnostics with source locations

**Objective:** Errors that look professional. Line, column, source excerpt, caret.

**What to figure out:**

- What's the minimum useful `Diagnostic` shape? Code, range, message — anything else?
- How do you render the source snippet — show the whole line? N lines of context? Where does the caret go for a multi-line range?
- How do you turn a `Range<u32>` byte range into `(line_start, col_start, line_end, col_end)` efficiently? (Hint: a `Vec<u32>` of line-start offsets, binary search.)

**Design questions:**

- Roll your own pretty-printer (~30 lines) or pull in `ariadne`/`miette`? What's the dependency cost vs. the time saved? At skillet's scale, both are defensible — but only one teaches you anything.
- Sort diagnostics by source order before printing, or by severity, or by file? Pick one consistently.

**Gotchas:**

- Multi-byte UTF-8 in the source means a byte offset doesn't always map cleanly to a "column" — define column as bytes or as Unicode code points, and document the choice. (Bytes is faster and matches what most editors actually show.)
- Diagnostic equality: two diagnostics with the same code, range, and message should be equal. Useful for snapshot tests.

**Acceptance:**

- An unknown ref in a `.pan` file produces output like:
  ```
  error[ref-not-found]: reference 'foo/bar.md' does not exist
   --> src/skills/my-skill/my-skill.pan:14:8
     |
  14 | See ref::foo/bar.md for details.
     |        ^^^^^^^^^^^
  ```
- The line and column are correct. Test with a multi-byte UTF-8 file to verify your offset math.

**Reading:**
- `rustc`'s `EmitterWriter` is the gold standard but huge. Look at the *output*, not the code, to see what good looks like.
- `ariadne`'s README has a screenshot — that's the bar.

---

## Stage 6 — Lockfile and incremental builds

**Objective:** Re-running `skillet build` on an unchanged project does (almost) no work.

**What to figure out:**

- What goes in the lockfile? Per-file hashes, sure — but also: skillet version, resolved settings hash, full ref inventory? Each addition has a cost (more reasons to invalidate) and a benefit (more reasons to trust a cache hit).
- How do you compute the hash? `seahash` (fast, non-crypto), `xxhash` (fast), `sha256` (slow but standard)? What property do you actually need?
- What's the comparison strategy on startup? Hash every file and compare, or trust mtime first and only hash on mismatch? (Hint: hashing tens of small files takes microseconds; mtime is a false optimization at skillet's scale. Hash everything.)

**Design questions:**

- Is the lockfile a build *output* (gitignored, regenerated each run) or a build *input* (committed, used to detect drift)? Skillet's existing model treats it as an output that ships with the compiled skill; preserve that.
- Lockfile format: TOML, JSON, your own line-based format? What's optimized for human review during code review?

**Gotchas:**

- Hash map iteration order is non-deterministic in stable Rust. If you serialize a `HashMap` into the lockfile, output will vary across runs and the lockfile becomes useless as a cache key. Use `BTreeMap` or sort explicitly.
- The lockfile must be byte-stable across platforms. Normalize line endings to LF before hashing file contents; otherwise Windows users get different hashes than Linux users for the same logical content.

**Acceptance:**

- `skillet build && skillet build` — the second invocation produces zero output and exits in <10 ms.
- Touching any file (mtime only) — second invocation produces zero output (because content is unchanged).
- Editing any byte of any file — second invocation rebuilds only the affected skill.

**Reading:**
- Cargo's `fingerprint` module is the deepest treatment of this problem. Way too much for skillet, but worth a skim to understand the tradeoffs (the [cargo book's "Build Cache" chapter](https://doc.rust-lang.org/cargo/guide/build-cache.html) is more accessible).

---

## Stage 7 — Profile, then maybe parallelize

**Objective:** Know whether parallelism would help before adding it.

**What to figure out:**

- What does `time skillet build` actually show on a real project? Break it into walk, parse, compile, write phases. Where's the bottleneck?
- If the whole thing finishes in <50 ms on a real project, you're done. Move on. Don't add rayon.
- If it's >100 ms and the bottleneck is per-file compilation across many files, then rayon. If the bottleneck is something else (slow ref-resolution? disk IO?), parallelism won't help.

**Gotchas:**

- `cargo build --release` before you measure. Debug profile numbers will lie to you about where time goes.
- Allocator pressure is invisible in wall time but visible in `cargo flamegraph` — if `malloc` shows up at the top, you have a `String` somewhere you should make a `&str`.

**Acceptance:**

- A short note (in this file, in a "Profile Results" section) recording what you measured and what you decided. If you didn't add rayon, write down why. Future-you will thank you.

**Reading:**
- `cargo flamegraph` documentation. Get comfortable with it; profiling without flamegraphs is guessing.

---

## Stage 8 — Stretch goals

Things to attempt once the core is solid, in rough order of value:

- A `skillet check` command that runs validation without writing output (useful for CI).
- A `--watch` mode using the `notify` crate, debounced ~100 ms.
- LSP integration — once you have an AST with positions and a diagnostic type with ranges, the LSP layer is small. The [`tower-lsp`](https://github.com/ebkalderon/tower-lsp) crate is the standard starting point.
- A formatter. This is a big undertaking — see Part 3 of [[Rust Linters and Formatters - Skillet Design Notes]] for the design space (Wadler IR via dprint, or Prettier-style IR via a fork of `biome_formatter`). Don't attempt until you have a real reason.

---

## Meta-advice

Things that will save you time across all stages:

- **Snapshot testing with [`insta`](https://insta.rs/)** is overpowered for the parser and the compiler. Capture the AST debug-output for an input, accept it, and then any regression shows up as a diff. The `cargo insta review` workflow is genuinely pleasant.
- **`#[derive(Debug)]` on every AST node**, every diagnostic, every intermediate type. Future debugging is dominated by `dbg!()` calls.
- **Don't fight the borrow checker on lifetimes if you've been at it for more than 20 minutes.** Switch to owned `String` / `Range<u32>` for that node, ship it, and revisit later. The performance loss at skillet's scale is invisible; the time loss to lifetime puzzles is real.
- **Resist `tokio`.** Skillet is CPU-bound on tiny inputs. Async adds complexity for zero gain.
- **When you finish a stage, write the diff up in a one-paragraph note here.** What was easier than expected, what was harder, what you'd do differently. This is how you cash in the learning.

Good luck. The architecture in [[Right-Sized Architecture]] is the destination; this note is the map. Have fun.
