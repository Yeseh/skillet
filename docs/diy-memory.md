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

## Current stage: Stage 1 complete — ready for Stage 2

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

## Stage 2 — 🔜 Not started

Next up: tokens and the lexer. Key design questions to prime:

- What *are* the tokens of `.pan`? What counts as a token vs opaque body text?
- How are the special constructs (`{{> name }}`, `var::FOO`, `ref::path`,
  `cmd::name`, `skill::name`) tokenised — whole tokens or multi-token sequences?
- Streaming iterator vs `Vec<Token>`?
- Error tokens vs aborting the stream?

The professor should open Stage 2 by asking the user to define what a `.pan`
token even is before any code is written.
