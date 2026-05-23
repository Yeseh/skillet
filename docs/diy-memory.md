---
created: 2026-05-22
tags: [skillet, diy, progress, learning]
related:
  - "[[docs/diy.md]]"
---

# Skillet DIY — Session Memory

Progress log for the hands-on build guide in `docs/diy.md`. A new professor
session should read this file first to know where the user is and what they've
already worked through.

---

## Current stage: Stage 2 complete — ready for Stage 3

---

## Stage 0 — ✅ Done (pre-existing)

The workspace was already set up before the DIY guide was written. The current
layout differs slightly from the guide's proposal:

```
skillet/
  crates/
    skillet/            # lib — core logic (parse, compile, lint, etc.)
    skillet-cli/        # bin — CLI frontend
    skillet-compiler/   # lib — new home for the Stage 1+ architecture
```

`skillet-ast` and `skillet-parser` do not exist yet as separate crates.
`skillet-compiler` is the staging ground for the new architecture.
`clippy::pedantic` is enabled at the workspace level.

---

## Stage 1 — ✅ Complete

**File:** `crates/skillet-compiler/src/lib.rs`

### What was built

`PanSource` — owns a `.pan` file's bytes and can answer "what line/column is
byte offset N?" in O(log n) via a pre-built line-start table.

### Key design decisions made

| Decision | Choice | Reasoning |
|---|---|---|
| Source buffer type | `Box<str>` | No ref-counting needed; single owner per build pipeline run. `String` wastes capacity; `Arc<str>` adds atomic overhead with no benefit at this scale. |
| Path field type | `Option<PathBuf>` | More idiomatic than `Option<String>`; avoids lossy UTF-8 conversion on non-ASCII paths. |
| `from_path` parameter | `&Path` | More flexible than `&PathBuf`; `PathBuf` coerces to `Path`, so callers aren't constrained. |
| Offset unit | **Byte offsets (`u32`)** | Faster than char indices (no UTF-8 decode); matches what editors use internally. Committed to in doc comments. |
| Line-start table | `Vec<u32>` built at construction time | One allocation up front; `location_at` is then a binary search + subtraction — no repeated scanning. |
| Column numbering | 1-based | Matches user-facing convention (line 1, column 1 for the first character). |

### Key concepts the user worked through

- **Line-start offsets vs newline positions** — the subtle bug of storing `\n`
  position instead of `\n + 1` (the actual start of the next line). The
  original implementation had two bugs that cancelled each other out:
  storing `\n` position (off by -1) and omitting `+1` in the column formula
  (off by +1). Both were identified and fixed.

- **`partition_point` vs `binary_search`** — `partition_point` with
  `|&start| start <= offset` is the clean fit: finds the last line-start
  ≤ the query offset without needing to handle `Err` variants.

- **The invariant that makes it safe** — every line-start is ≤ any offset on
  that line, by construction. This means `partition_point` always returns a
  valid index ≥ 1, so `(line_idx - 1)` never underflows (assuming a well-formed
  source with `offsets[0] = 0` always present).

### Final shape of the API

```rust
pub struct SourceLocation { pub line: u32, pub column: u32 }

pub struct PanSource { /* path, src: Box<str>, offsets: Vec<u32> */ }

impl PanSource {
    pub fn new(src: String, path: Option<PathBuf>) -> Self
    pub fn from_path(path: &Path) -> io::Result<Self>
    pub fn as_str(&self) -> &str
    pub fn location_at(&self, offset: u32) -> SourceLocation
}
```

### Remaining polish (not blocking Stage 2)

- `SourceLocation` fields should be `pub` so external callers can read them
  (currently works in tests because `mod tests` is a child module, but breaks
  outside the crate).
- Doc comment on `PanSource` (or `location_at`) should state the byte-offset
  contract explicitly.

---

## Stage 2 — ✅ Complete

**File:** `crates/skillet-compiler/src/lex.rs`

### What was built

`Lexer<'a>` — a hand-written, zero-allocation lexer that turns a `&str` into
a `Vec<Token>`. Each `Token` is a `(TokenKind, Range<u32>)` — no heap
allocation per token. The lexer handles all `.pan` structural constructs and
produces correct ranges for every token kind.

### Key design decisions made

| Decision | Choice | Reasoning |
|---|---|---|
| Token shape | `TokenKind` enum + `Range<u32>` | Zero allocation; slices back into source via range when text is needed. |
| Fragment delimiters | `{>` open, `<}` close | Simplified from the original `{{> }}` syntax; easier to lex unambiguously. |
| Ref syntax | Backtick-prefixed only: `` `ref::foo` `` | Naked `ref::foo` in prose is BodyText; only backtick-wrapped refs are structural. |
| Tick-context detection | Check `src[start_pos - 1] == b'\`'` in `tick_prefixed_ref_token` | No state needed; the previous byte tells us if we're inside a backtick context. |
| Double-tick escape | `` ``...`` `` suppresses structural token detection | TickDouble pushed silently, then `make_body` called directly — escaped content never reaches the keyword-detection arm. |
| Code fence | Triple backtick absorbed as single `BodyText` | Delimiters included in range; `make_body` detects and skips internal triple-backtick sequences. |
| Multi-token constructs | `::` and `FragmentOpen`/`FragmentClose` pushed directly inside match arms | Allows one return value per loop iteration while still emitting multiple tokens for `::`-prefixed refs and fragments. |
| Terminator handling | `pos -= 1` after consuming terminator in helpers | Leaves the terminating byte for the outer loop; no data loss. |
| `Vec<Token>` over streaming iterator | Materialise everything | Parser needs arbitrary lookahead; `Vec` is cheap at skillet's scale and dramatically simplifies the parser. |

### Key concepts the user worked through

- **`start_pos` capture after `next()`** — `next()` advances `pos` before
  returning, so `start_pos = (self.pos - 1) as u32` is required. An off-by-one
  here would shift every token range by one byte.

- **`peek` vs `next` for terminators** — helpers initially consumed the
  terminating byte via `next()`, causing it to disappear from the stream. Fixed
  with `self.pos -= 1` before `break` to "unconsume" the terminator.

- **`slice.get(index).copied()`** — the idiomatic zero-panic bounds-checked
  byte access pattern; replaces manual `if pos < len` guards.

- **`starts_with` for keyword detection** — checking `src[start_pos..].starts_with(b"ref::")` is simpler and clearer than a character-by-character scan loop.

- **`len - 3` advancement for keywords** — constants include `::` (e.g.
  `b"ref::"` len=5), but only the keyword bytes should be consumed; `::` must
  be left for the main loop's `b':'` arm. Advancing by `len - 3` (not `len - 2`)
  leaves both colons intact.

- **Two-token emission in one iteration** — `DoubleColon` and `FragmentOpen`
  are pushed directly to `tokens` inside the match arm, then the arm returns a
  second token (`RefValue` / fragment id) as its expression value. The `println`
  at the bottom of the loop only captures the return-value token, which confused
  debugging until the full `tokens` vec was inspected.

### Final token inventory

```
BodyText      — prose, code fences (with delimiters), and anything not structural
Tick          — single backtick (inline code marker / ref context opener)
TickDouble    — double backtick (escape sequence delimiter)
DoubleColon   — `::` separator between ref type and ref value
FragmentOpen  — `{>` fragment insertion opener
FragmentClose — `<}` fragment insertion closer
RefValue      — the value after `::` or the name inside a fragment
RefReference  — `ref` keyword
RefSKill      — `skill` keyword
RefAgent      — `agent` keyword
RefCmd        — `cmd` keyword
Invalid       — reserved for future error token use
```

### What was easier than expected

- The `make_body` / `make_ref_value` / `make_fragment_id` helper pattern clicked
  quickly once the "unconsume terminator" rule was established.
- `starts_with` on byte slices made keyword detection trivial.

### What was harder than expected

- The two-token-per-iteration pattern (DoubleColon + RefValue, FragmentOpen +
  RefValue) required breaking the clean "one token per loop iteration" rule.
  Took a few iterations to settle on pushing directly to `tokens` inside the arm.
- Debugging was hampered by `println!` only showing return-value tokens, not
  tokens pushed inside arms. Lesson: print the full vec after tokenizing, not
  inline during the loop.

---

## Stage 3 — ✅ Complete

**File:** `crates/skillet-compiler/src/parse.rs`

### What was built

`PanParse<'a>` — borrows from a `PanSource` and produces a flat `Vec<Node>` plus
a `Vec<ParseError>`. The parser walks the `Vec<Token>` from the lexer using a
`Peekable` iterator, consuming tokens in sequence and building typed nodes.

### Key design decisions made

| Decision | Choice | Reasoning |
|---|---|---|
| AST shape | Flat `Vec<Node>`, not a tree | `.pan` has no nesting; a flat list is sufficient and simpler |
| Ranges | On every node (`source_range: Range<u32>`) | Side-table approach would require reconstructing ranges from token spans at every diagnostic call site |
| Error handling | Collect into `Vec<ParseError>`, always return `PanParse` | A linter that aborts on first error is half a linter |
| Return type | `PanParse` struct (nodes + errors + path), no `Result` | Parsing always produces something useful; `Result` was a false frame |
| Error message type | `ParseErrorKind` enum (not `String`) | Lets the compiler match on error kind; dynamic messages would be `String` not `Box<str>` |
| Borrow approach | `PanParse<'a>` borrows `src: &'a str` from `PanSource` | Avoids cloning; lifetime is clear; moved `get_source_string` to an `impl` method to avoid closure borrow conflicts |
| Recovery boundary | Advance to next `Tick` or `FragmentOpen` via `recover()` | These are the only valid entry points for structural nodes in the outer loop |
| Ref parsing | `make_ref_node` helper takes `RefKind` parameter | All six ref kinds share identical token-consumption logic; only `RefKind` varies |

### Node inventory

```rust
pub enum Node {
    Ref      { kind: RefKind, value: String, source_range: Range<u32> },
    Fragment { value: String, source_range: Range<u32> },
    RefSuspect  { source_range: Range<u32> },   // `backtick body` — lint candidate
    EscapedBody { source_range: Range<u32> },   // ``double backtick body``
    Body        { source_range: Range<u32> },   // plain prose
}
```

### Key concepts the user worked through

- **AST carries semantics, not tokens** — early sketches listed token sequences
  (tick + keyword + doublecolon + value + tick); the insight was that the compiler
  only needs `RefKind` + value, not the surrounding syntax.

- **Side-table vs on-node ranges** — rejected side-table because every diagnostic
  call site would need to reconstruct the range from constituent tokens.

- **`Result` vs always-succeed struct** — `Result<Vec<Node>, Vec<ParseError>>`
  was the first instinct; corrected to `PanParse` because partial results and
  errors coexist during error recovery.

- **Closure borrow conflict** — initial `get_source_string` closure captured
  `self.src` immutably, conflicting with `self.nodes.push()` mutable borrows.
  Fixed by moving it to an `impl` method.

- **`pub` on enum variant fields** — Rust doesn't allow `pub` on struct-variant
  fields; visibility is controlled by the enum's own `pub`.

- **Peek-before-commit** — the `make_ref_node` helper originally consumed the
  ref keyword token before checking for `DoubleColon`. Fixed to peek first; if
  no `DoubleColon` follows, nothing is consumed and the outer loop handles the
  tokens naturally.

- **Fallthrough after error + recover** — multiple arms pushed an error and called
  `recover()` but then fell through to `unwrap()` on the next token. Required
  explicit `return` / `continue` after each recovery path.

- **Double-emit bug** — `Tick => BodyText` arm peeked `p` but didn't consume it,
  causing the `BodyText` to be emitted twice (once in merged range, once on next
  iteration). Fixed by calling `next()` after the peek.

- **`SuspectedRef` / `RefSuspect` variant** — backtick-wrapped plain text
  (`` `some-thing` ``) is structurally a candidate for a typed ref. Emitted as
  its own node so the compiler can issue a lint suggestion.

- **Syntax mismatch with existing `.pan` files** — example files use `{{> name }}`
  (old syntax); lexer uses `{>` / `<}` (new syntax). Examples are integration
  tests with the full toolchain and must not change until the whole pipeline
  is swapped over. New-syntax unit tests are the correct test surface for the
  parser.

### What was easier than expected

- The `make_ref_node` extraction was clean once the peek-before-commit fix was in.
- `recover()` was five lines once the safe-boundary invariant was clear.

### What was harder than expected

- The fallthrough-after-error pattern bit multiple times across `TickDouble`,
  `FragmentOpen`, and `make_ref_node`. Each arm needed independent review.
- The closure borrow conflict required a small structural rethink (impl method)
  rather than a local fix.

### Test coverage

29 parser tests, all green. Covers all node kinds, all error kinds, source range
correctness for delimiters, and multi-construct documents.

---

## Stage 4 — 🔜 Not started

Next up: compiler / visitor. Key design questions to prime:

- Plain `match` or `Visitor` trait?
- Where does fragment expansion happen — inline during walk, or a separate pass?
- How does the `CompileContext` get built, and when?
- Token counting: as-you-emit or end-pass?
