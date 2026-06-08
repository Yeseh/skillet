# Handoff: Implement `compiler::compile()`

The refactor is structurally complete and compiles. The remaining work is implementing
the actual compile pass using the new architecture. Nine integration tests in
`crates/skillet-cli/tests/build_tests.rs` define the expected behaviour.

---

## Architecture

The intended design has three layers:

```
CLI (build.rs)
  → skill.compile(&ws)         via CompiledArtefact trait
    → compiler::compile(ws, source)   raw AST walk, returns text + diagnostics
```

`Workspace` is the single source of truth for everything the compiler needs.
The CLI resolves it once, passes it everywhere, and only handles I/O (writing files).

---

## Step 1: Absorb config data into `Workspace`

`Workspace` currently holds fragments but not the config fields the compiler needs.
Add them so `compile(&ws)` has everything without taking a `&SkilletConfig` parameter.

**In `crates/skillet/src/workspace/mod.rs`:**

Add to the `Workspace` struct:

```rust
/// Workspace variable substitutions from `[vars]` in `skillet.toml`.
pub vars: BTreeMap<String, String>,
/// Declared environment variables with defaults from `[env]`.
pub env: BTreeMap<String, config::EnvVar>,
/// Tiktoken encoding name used for all token counting.
pub tokenizer: String,
```

Populate in `Workspace::resolve()` by cloning from `cfg`:

```rust
Ok(Self {
    // existing fields ...
    vars: cfg.vars.clone(),
    env: cfg.env.clone(),
    tokenizer: cfg.build.tokenizer.clone(),
})
```

Add the necessary imports (`BTreeMap`, `crate::config`).

---

## Step 2: Give `compiler::compile()` a return type

**In `crates/skillet/src/compiler/compile.rs`**, replace the stub:

```rust
pub fn compile(ws: &Workspace, source: &PanSource) {
}
```

with a real signature and return type:

```rust
pub struct CompileOutput {
    /// Compiled markdown text (everything after the frontmatter).
    pub text: String,
    /// All diagnostics. Errors mean the build should fail; warnings do not.
    pub diagnostics: Vec<CompileDiag>,
    /// Fragment ids spliced in, in first-use order (deduplicated).
    pub fragments_used: Vec<String>,
    /// Tiktoken count over `text`.
    pub activation_tokens: u32,
    /// Token count over `"{name} {description}"` from the frontmatter.
    pub discovery_tokens: u32,
}

pub fn compile(ws: &Workspace, source: &PanSource) -> CompileOutput {
    todo!()
}
```

---

## Step 3: Implement `compiler::compile()`

The function receives the full source (frontmatter + body). It needs to:

1. Split off the body (everything after the closing `---` of frontmatter).
2. Parse the body into an AST via `PanParse`.
3. Walk the AST, building an output string and accumulating diagnostics.
4. Count tokens over the assembled output string.
5. Compute `discovery_tokens` from the frontmatter name + description.

### 3a. Split frontmatter from body

Use `crate::parse::parse_frontmatter(source.as_str())` to extract the frontmatter.
The body starts after the second `---\n`. A simple split:

```rust
let raw = source.as_str();
let body = if raw.starts_with("---") {
    // find the closing ---
    raw[3..].find("\n---").map(|i| &raw[i + 7..]).unwrap_or(raw)
} else {
    raw
};
```

Or use `gray_matter` (already a dependency) — it's already used in `parse.rs`.

### 3b. Parse and walk the AST

```rust
let pan_source = PanSource::new(body.to_string());
let mut parser = PanParse::new(&pan_source);
parser.parse();

let mut out = String::with_capacity(body.len());
let mut diagnostics: Vec<CompileDiag> = Vec::new();
let mut fragments_used: Vec<String> = Vec::new();

for node in &parser.nodes {
    match node {
        Node::Body { source_range } => {
            out.push_str(&body[source_range.start as usize..source_range.end as usize]);
        }
        Node::EscapedBody { source_range } => {
            // ``content`` → `content`
            let raw = &body[source_range.start as usize..source_range.end as usize];
            out.push('`');
            out.push_str(&raw[2..raw.len() - 2]);
            out.push('`');
        }
        Node::RefSuspect { source_range } => {
            // unrecognised backtick — pass through verbatim
            out.push_str(&body[source_range.start as usize..source_range.end as usize]);
        }
        Node::MarkdownLink { text, target, source_range } => {
            // pass through verbatim
            out.push_str(&body[source_range.start as usize..source_range.end as usize]);
        }
        Node::Fragment { value, source_range } => {
            // interpolate from ws.fragments (see below)
        }
        Node::Ref { kind, value, source_range } => {
            // resolve typed ref (see below)
        }
    }
}
```

### 3c. Fragment interpolation

```rust
Node::Fragment { value, source_range } => {
    let id = value.trim();
    let loc = pan_source.location_at(source_range.start);
    if let Some(reason) = ws.fragments.poisoned.get(id) {
        diagnostics.push(CompileDiag {
            severity: DiagSeverity::Error,
            message: format!("cannot expand fragment '{}': {}", id, reason),
            line: loc.line, col: loc.column,
        });
    } else if let Some(content) = ws.fragments.rendered.get(id) {
        out.push_str(content);
        if !fragments_used.contains(&id.to_string()) {
            fragments_used.push(id.to_string());
        }
    } else {
        diagnostics.push(CompileDiag {
            severity: DiagSeverity::Error,
            message: format!("fragment '{}' not found", id),
            line: loc.line, col: loc.column,
        });
    }
}
```

`ws.fragments.poisoned` is populated by `render_fragments()` when a fragment contains
a nested `{> ... <}` include — this is already working.

### 3d. Ref resolution (`emit_ref`)

`known_files` is per-skill (all files under the skill's `src_dir`). It is computed
by `Skill::compile()` before calling this function, then passed in as a parameter.
Add it to `compile()`'s signature:

```rust
pub fn compile(
    ws: &Workspace,
    source: &PanSource,
    known_files: &HashSet<String>,
) -> CompileOutput
```

Ref dispatch:

```rust
Node::Ref { kind, value, source_range } => {
    let loc = pan_source.location_at(source_range.start);
    match kind {
        RefKind::Reference => {
            if !known_files.is_empty() && !known_files.contains(value.as_str()) {
                diagnostics.push(CompileDiag {
                    severity: DiagSeverity::Error,
                    message: format!("ref path not found: '{}'", value),
                    line: loc.line, col: loc.column,
                });
            }
            out.push('`'); out.push_str(value); out.push('`');
        }
        RefKind::Cmd => {
            let cmd = value.split_whitespace().next().unwrap_or(value);
            if !workspace::is_on_path(cmd) {
                diagnostics.push(CompileDiag {
                    severity: DiagSeverity::Warning,
                    message: format!("command '{}' not found on PATH", cmd),
                    line: loc.line, col: loc.column,
                });
            }
            out.push('`'); out.push_str(value); out.push('`');
        }
        RefKind::Skill => {
            if !ws.skills.is_empty() && !ws.skills.contains_key(value.as_str()) {
                diagnostics.push(CompileDiag {
                    severity: DiagSeverity::Error,
                    message: format!("skill '{}' not found in workspace", value),
                    line: loc.line, col: loc.column,
                });
            }
            out.push('`'); out.push_str(value); out.push('`');
        }
        RefKind::Agent | RefKind::Path | RefKind::Url => {
            out.push('`'); out.push_str(value); out.push('`');
        }
        RefKind::Var => match ws.vars.get(value.as_str()) {
            Some(v) => out.push_str(v),
            None => diagnostics.push(CompileDiag {
                severity: DiagSeverity::Error,
                message: format!("var '{}' not declared in [vars]", value),
                line: loc.line, col: loc.column,
            }),
        },
        RefKind::Env => match ws.env.get(value.as_str()) {
            Some(e) => {
                let resolved = std::env::var(value).unwrap_or_else(|_| e.default.clone());
                out.push_str(&resolved);
            }
            None => diagnostics.push(CompileDiag {
                severity: DiagSeverity::Error,
                message: format!("env '{}' not declared in [env]", value),
                line: loc.line, col: loc.column,
            }),
        },
    }
}
```

`workspace::is_on_path()` already exists in `workspace/mod.rs`.

### 3e. Token counting

```rust
let activation_tokens = crate::tokens::count_tokens(&out, &ws.tokenizer);

let discovery_tokens = {
    let fm = crate::parse::parse_frontmatter(source.as_str())
        .ok().flatten();
    let text = fm.map(|f| format!(
        "{} {}",
        f.name.unwrap_or_default(),
        f.description.unwrap_or_default()
    )).unwrap_or_default();
    crate::tokens::count_tokens(&text, &ws.tokenizer)
};
```

---

## Step 4: Implement `Skill::compile()` via the trait

**In `crates/skillet/src/workspace/skill.rs`**, fill in the `todo!()`:

```rust
impl CompiledArtefact for Skill {
    fn compile(&self, ws: &super::Workspace) -> anyhow::Result<Option<String>, Vec<crate::compiler::CompileDiag>> {
        use walkdir::WalkDir;

        let source_text = std::fs::read_to_string(&self.source_path)
            .map_err(|e| vec![/* surface as a diag or propagate */])?;

        let pan_source = crate::compiler::PanSource::new(source_text);

        let known_files: std::collections::HashSet<String> = WalkDir::new(&self.src_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| {
                e.path()
                    .strip_prefix(&self.src_dir)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            })
            .collect();

        let output = crate::compiler::compile::compile(ws, &pan_source, &known_files);

        let errors: Vec<_> = output.diagnostics.iter()
            .filter(|d| d.severity == crate::compiler::DiagSeverity::Error)
            .cloned()
            .collect();

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Some(output.text))
    }

    fn check(&self, ws: &super::Workspace) -> anyhow::Result<Vec<crate::compiler::CompileDiag>> {
        // Same as compile() but skip the Ok/Err split — just return all diagnostics.
        todo!()
    }
}
```

Note: `compile()` returns `Result<Option<String>, Vec<CompileDiag>>`. `None` means
"nothing to emit" (not currently used for skills, but kept for the trait's generality).

---

## Step 5: Wire the CLI

**In `crates/skillet-cli/src/build.rs`**, `compile_one_skill` currently:

```rust
fn compile_one_skill(skill: &Skill, cfg: &SkilletConfig, ws: &Workspace, _lockfile: &mut Lockfile) -> Result<()> {
    let source_content = std::fs::read_to_string(&skill.source_path)?;
    let pan_source = PanSource::new(source_content);
    skillet::compiler::compile::compile(ws, &pan_source);  // stub
    let _source_hash = hash_file(&skill.source_path)?;
    Ok(())
}
```

Replace with:

```rust
fn compile_one_skill(skill: &Skill, ws: &Workspace, lockfile: &mut Lockfile) -> Result<()> {
    use skillet::workspace::CompiledArtefact;

    let text = skill.compile(ws).map_err(|diags| {
        for d in &diags {
            eprintln!("{}", render_diag(skill, d));
        }
        anyhow::anyhow!("compile errors in '{}'", skill.name)
    })?;

    let text = text.unwrap_or_default();

    std::fs::create_dir_all(&skill.target_dir)?;
    let output_path = skill.target_dir.join("SKILL.md");
    std::fs::write(&output_path, &text)?;

    let source_hash = hash_file(&skill.source_path)?;
    let compiled_hash = hash_bytes(text.as_bytes());

    let old_minhash = lockfile
        .skills.get(&skill.name)
        .filter(|e| e.compiled_hash == compiled_hash)
        .map(|e| e.minhash.clone())
        .unwrap_or_default();

    lockfile.skills.insert(skill.name.clone(), ArtefactEntry {
        source_hash,
        compiled_hash,
        discovery_tokens: 0,   // TODO: thread through from CompileOutput
        activation_tokens: 0,  // TODO: thread through from CompileOutput
        transitive_tokens: 0,
        fragments_used: vec![],
        refs: ArtefactRefs::default(),
        minhash: old_minhash,
    });

    Ok(())
}
```

Token counts and `fragments_used` are currently in `CompileOutput` but `Skill::compile()`
only returns the text. You have two options:

- **Option A:** Change the trait return type to carry a richer result struct.
- **Option B:** Keep the trait minimal and have `compile_one_skill` call
  `compiler::compile()` directly (after trait compilation succeeds) to get the full
  `CompileOutput`. The trait call validates; the direct call extracts metrics.

Pick whichever fits the design intent.

Also update the call site in `run()` to drop the `cfg` argument:

```rust
compile_one_skill(skill, &ws, &mut lockfile)?;
```

Remove `cfg: &SkilletConfig` from `compile_one_skill`'s signature — config data now
lives on `ws`.

---

## Diagnostic format required by tests

From `build_reports_ref_errors_with_source_location`:

```
[error] my-skill ref path not found: 'missing.txt' (path:6:5)
```

A helper in `build.rs`:

```rust
fn render_diag(skill: &Skill, d: &CompileDiag) -> String {
    let level = match d.severity {
        DiagSeverity::Error => "error",
        DiagSeverity::Warning => "warning",
    };
    format!(
        "[{level}] {} {} ({}:{}:{})",
        skill.name, d.message, skill.source_path.display(), d.line, d.col
    )
}
```

Warnings should also be printed but must **not** cause a non-zero exit.
Print them after the `skill.compile()` call succeeds — you'll need to expose them
from the trait result or call `compiler::compile()` directly for warnings.

---

## Integration tests

Run with:

```
cargo test --test build_tests
```

All 9 tests are in `crates/skillet-cli/tests/build_tests.rs`. They drive the real
CLI binary end-to-end. The test file is the authoritative spec — read it directly.
