# Skillet: Agentic Application Development Toolkit

Skillet is a development toolkit for building and deploying agentic applications through composable skill bundles.

## Overview

Skillet provides two complementary paths:
1. **Traditional Skills** — Text-based `.pan` → `SKILL.md` compilation with token budgeting, linting, and reference management
2. **Bundle Skills** — WebAssembly-based deployable skill bundles with embedded instructions, tools, and references

## Core Concepts

### What is a Skill?

A skill is a functional capability bundle consisting of:
- **Instructions** — Markdown guidance for LLM agents
- **Knowledge** — Reference materials (documentation, examples)
- **Tools** — Executable scripts or functions

Skills map to functional domain slices in applications (e.g., `order-management`, `customer-profile`, `inventory-sync`).

### Agents vs Skills

**Agents are:**
- Instructions + Knowledge (RAG/memory) + Tools (local/MCP/A2A)
- Running in a loop (runtime component)

**Skills are:**
- Instructions + Knowledge (references) + Tools (scripts)
- Stateless capability bundles

The key insight: **Skills are the meat-and-potatoes of what an agent is.** An agent is essentially a runtime that orchestrates skill activation based on context.

## Problems Solved

### Skill Authoring
- ✅ Reference checking for agentic artifacts, scripts, files
- ✅ Token cost tracking (discovery vs activation budgets)
- ✅ Evaluation tools and duplication detection
- ✅ Bundling application code as callable tools

### Shared Code Between Skills
**Problem:** Sharing common logic (logging, auth, auditing) across skills currently requires:
- Shipping a separate CLI application
- Manual vendoring/copying of skill libraries before publishing

**Solution:** WASI modules enable composing shared code libraries directly into skill bundles.

### Skill Runtime Burden
**Problem:** Installing skill runtimes (Python, Node) falls to the consumer when locally run.

**Solution:** WASI modules compile to portable sandboxed WebAssembly — no language runtime installation required.

### Sandboxing & Permissions
**Problem:** Skills need controlled access to filesystems, network, and commands.

**Solution:** WASI permission system with pre-opened directories, allowed network paths, and explicit command grants.

## Architecture

### Traditional Skills (`.pan` → `SKILL.md`)

**Directory Structure:**
```
src/skills/
  my-skill/
    my-skill.pan          ← Source with frontmatter, fragments, typed refs
  _fragments/
    common-header.fragment.pan
```

**Build Pipeline:**
- Fragment expansion (`{{> fragment-name }}`)
- Variable/env substitution (`var::`, `env::`)
- Typed ref validation (`ref::`, `cmd::`, `skill::`)
- Token counting (discovery + activation costs)
- Lockfile generation with hashes, ref inventory

**Output:** `skills/my-skill/SKILL.md`

### Bundle Skills (WASI Modules)

**Directory Structure:**
```
src/
  order-management/
    instructions.pan         ← Instructions (name inferred from directory)
    references/
      order-schema.pan       ← Embedded reference material
      api-guide.pan
    tools/
      create-order/          ← Individual tool as Rust crate
        src/
          main.rs
        Cargo.toml
      update-order/
        src/
          main.rs
        Cargo.toml
```

**Build Pipeline (`skillet bundle`):**
1. Compile `instructions.pan` → markdown (existing pipeline)
2. Compile `references/*.pan` → markdown
3. Build each tool crate to `wasm32-wasip2`
4. Compose tool components via `wasm-tools component compose`
5. Generate root component with embedded instructions, references, dispatch logic
6. Output single `.wasm` artifact

**Output:** `skills/order-management/order-management.wasm`

---

# Design Decisions (2026-05-21 Session)

## 1. Deployment Architecture

**Decision:** Single WASM component per skill with everything embedded (instructions, references, tools).

**Rationale:** Simplifies deployment — one artifact to load, no external dependencies. Instructions and references become part of the binary, enabling offline operation and consistent distribution.

## 2. Build Pipeline Separation

**Decision:** Cargo feature-gated `skillet bundle` command, separate from `skillet build`.

**Rationale:** Keeps traditional text-only skill workflow unchanged. WASI toolchain complexity (Rust, wasm32-wasip2 target, wasm-tools) only required for bundle users. Two distinct paths coexist cleanly.

## 3. Project Structure

**Directory Layout:**
```
src/
  <skill-name>/              ← Directory name IS the skill name
    instructions.pan         ← Fixed filename (vs traditional <name>.pan)
    references/              ← Reference material (.pan files)
      ref1.pan
      ref2.pan
    tools/                   ← Presence signals bundle skill
      <tool-name>/           ← Each tool is a Rust crate
        src/
          main.rs
        Cargo.toml
```

**Detection Mechanism:** Structural — presence of `tools/` directory signals bundle skill.

## 4. Skill Naming Convention

**Decision:** Skill name inferred from directory, injected into compiled frontmatter.

**Rationale:** No duplication, no mismatch risk. `instructions.pan` frontmatter only needs `description:` and metadata — `name:` is derived from parent directory.

## 5. Composition Strategy

**Decision:** Per-tool WASM components composed via `wasm-tools component compose`.

**Status:** Research pending — need to validate composition mechanics for unified dispatch surface.

## 6. WIT Interface Contract

**Skill Interface:**
```wit
get-instructions: func() -> string
list-tools: func() -> list<tool-definition>
call-tool: func(name: string, params: string) -> result<string, string>
list-references: func() -> list<string>
get-reference: func(name: string) -> result<string, string>
```

**Host Interface:**
```wit
list-skills: func() -> list<skill-definition>
get-skill: func(name: string) -> skill
```

**Type Model:** JSON strings for `params` and results — maximizes flexibility, aligns with MCP/A2A conventions. Tools declare JSON schemas via ToolDefinition.

## 7. Progressive Disclosure

**SkillDefinition (Discovery Surface):**
- `name: string`
- `description: string`

**Mechanism:**
1. Host calls `list-skills()` → agent sees lightweight summaries
2. Agent decides relevance based on task
3. Host calls `get-skill(name)` → loads full instructions only when activated
4. Maps to existing `discovery_tokens` vs `activation_tokens` budget model

## 8. Tool Metadata Declaration

**Decision:** Rust macro/trait pattern inspired by `rmcp` library.

**Implementation:**
- Use `schemars` for JSON schema derivation from Rust types
- Proc-macro for tool registration (e.g., `#[tool(description = "...")]`)
- ToolDefinition includes name, description, JSON schema for params

**Validation Required:** Ensure MCP library's derive macros compile to `wasm32-wasip2`.

## 9. WASI Permission Model

**Declaration:** Both skill author and host operator participate (Android-style).

**Skill declares needed permissions in `skillet.toml`:**
```toml
[bundle.permissions]
dirs = ["./output", "./tmp"]
network = ["api.example.com"]
commands = ["git", "curl"]
```

**Host grants subset at load time.** Mismatches surface explicitly rather than failing silently at runtime.

## 10. References Handling

**Decision:** References embedded in binary.

**Pipeline:**
- `references/*.pan` compiled to markdown
- Embedded in WASM component
- `list-references()` returns available refs
- `get-reference(name)` returns content on-demand
- Agent-driven progressive disclosure for reference material

## 11. Bundle Output

**Output Path:** `skills/<skill-name>/<skill-name>.wasm` (matches existing convention)

**Artifact:** Single `.wasm` file. No sidecar manifest — host instantiates component to discover metadata via WIT interface.

## 12. Shared Core Library

**Decision:** Normal Cargo dependency, user-defined.

**Rationale:** Simpler than WASM composition for MVP. Skill tool crates add core library to `Cargo.toml` like any dependency. Core library must compile to `wasm32-wasip2`. Composition-based approach deferred until composition mechanics are validated.

## 13. Toolchain Invocation

**Decision:** `skillet bundle` shells out to `cargo build --target wasm32-wasip2`.

**Failure Mode:** Fast fail with actionable error if `wasm32-wasip2` target or `wasm-tools` not installed.

## 14. WIT File Ownership

**Decision:** Skillet ships canonical WIT files as a versioned crate (`skillet-wit`).

**Rationale:**
- Skillet owns the interface contract
- Skill tool crates depend on `skillet-wit`
- Host runtimes import the same WIT definition
- Interface stability is a toolchain concern, not per-project
- Enables clean version upgrades across ecosystem

## 15. Runtime Deployment Model

**Immediate Target (Option C):** Embedded runtime in calling agent's host.
- WASM bundle loaded directly by orchestrator (VS Code extension, Claude Code, custom agent loop)
- No separate server infrastructure
- Host embeds `wasmtime` library, loads skill components on-demand

**North Star (Option A):** Self-hosted server (`skillet serve`).
- Spins up local/cloud process that loads `.wasm` bundles from directory
- Exposes MCP/A2A/HTTP endpoints per bundle
- Teams deploy and share skills across organization

## 16. Build Pipeline Integration

**Decision:** One command — `skillet bundle` runs full pipeline internally.

**Steps:**
1. Compile `instructions.pan` → markdown (existing pipeline: fragments, vars, lint)
2. Compile `references/*.pan` → markdown
3. Build each tool crate → individual `.wasm` components
4. Compose via `wasm-tools component compose`
5. Write `skills/<name>/<name>.wasm`

**Intermediate Artifacts:** `SKILL.md` may be written for inspectability, but developer only runs one command.

## 17. WIT Interface Contract

**Decision:** Skillet ships canonical WIT files as versioned crate (`skillet-wit`).

**Rationale:** Toolchain owns interface stability. Skills and hosts upgrade together via crate versions.

## 18. Detection Mechanism

**Decision:** Structural detection — presence of `tools/` directory signals bundle skill.

**Rationale:** Convention over configuration. Clear visual signal in directory structure.

---

## Open Research Items

1. **WASM Component Composition:** Validate `wasm-tools component compose` mechanics — specifically whether per-tool components can be composed into a single skill component with unified dispatch surface.

2. **rmcp WASI Compatibility:** Verify MCP Rust library's proc-macros and schema derivation compile cleanly to `wasm32-wasip2`.

3. **Root Component Generation:** Determine whether composition requires a code-generated root component that exports the skill WIT interface and dispatches to composed tools.

---

## Example: Order Management Skill

**Traditional Skill (text-only):**
```
src/skills/order-management/
  order-management.pan
  reference/
    order-schema.md
    api-guide.md

→ builds to: skills/order-management/SKILL.md
```

**Bundle Skill (WASI):**
```
src/order-management/
  instructions.pan
  references/
    order-schema.pan
    api-guide.pan
  tools/
    create-order/
      src/main.rs
      Cargo.toml
    update-order/
      src/main.rs
      Cargo.toml

→ builds to: skills/order-management/order-management.wasm
```

**Host loads bundle:**
```rust
let skill = load_skill("skills/order-management/order-management.wasm")?;
let info = skill.list_tools(); // Discovery
let result = skill.call_tool("create-order", r#"{"order_id": "123", ...}"#)?;
```

---

## Decision Summary Table

| # | Decision | Choice |
|---|----------|--------|
| 1 | Deployment unit | Single WASM component — instructions baked in |
| 2 | CLI surface | Cargo feature-gated `skillet bundle`, separate from `skillet build` |
| 3 | Project structure | `src/<skill-name>/instructions.pan`, `references/`, `tools/<tool-name>/` |
| 4 | Binary granularity | One `.wasm` per skill |
| 5 | Composition mechanism | `wasm-tools component compose` *(research pending)* |
| 6 | WIT type model | JSON strings — `call-tool(name: string, params: string) -> result<string, string>` |
| 7 | Runtime | Embedded via `wasmtime`; north star is `skillet serve` |
| 8 | SkillDefinition | Name + description only |
| 9 | ToolDefinition | Rust macro/trait, `rmcp`-inspired, `schemars` for schema derivation |
| 10 | WASI permissions | Skill declares needed, host grants subset |
| 11 | References | Embedded in binary |
| 12 | Bundle output | Single `.wasm` → `skills/<name>/<name>.wasm` |
| 13 | Bundle detection | Structural — presence of `tools/` directory |
| 14 | Shared core library | Normal Cargo dependency, user-defined |
| 15 | Skill name | Inferred from directory, injected into compiled frontmatter |
| 16 | Toolchain invocation | `skillet bundle` shells out to `cargo build --target wasm32-wasip2` |
| 17 | WIT ownership | Skillet ships the canonical WIT files |
| 18 | Build integration | One command — `skillet bundle` runs full pipeline |




agents/
skills/
  bla1/
    /references
    /scripts
    /bla1.pan
