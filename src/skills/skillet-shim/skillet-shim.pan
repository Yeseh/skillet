---
name: skillet-shim
description: "Shim that delegates skill management to the skillet CLI."
---

# skillet-shim

This project uses `var::project_name` to manage AI agent skills. Use the `var::project_name` CLI for all skill-related tasks.

## Get full documentation

```bash
skillet skill print skillet
```

## Quick reference

| Command | Purpose |
|---|---|
| `cmd::skillet build` | Compile skill sources to SKILL.md |
| `cmd::skillet new <name>` | Scaffold a new skill |
| `cmd::skillet lint` | Check skills for quality issues |
| `cmd::skillet check` | Verify compiled output is up to date |
| `cmd::skillet budget` | Show token costs |
