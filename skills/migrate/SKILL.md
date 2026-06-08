---
name: migrate
description: "Migrate plain markdown SKILL.md files to a managed skillet workspace with typed refs and extracted fragments."
---

# migrate

Use this skill when converting a project's plain-markdown `SKILL.md` files into a fully managed `skillet` workspace with `.pan` sources, typed references, and shared fragments.

## Phase 1 — Adopt existing skills

Check whether the workspace is already initialised before doing anything else.

1. Look for `skillet.toml` in the project root.
2. If it is absent, run:
   ```
   skillet init --adopt
   ```
   `--adopt` copies every `skills/<name>/SKILL.md` it finds into
   `src/skills/<name>/<name>.pan` so the existing compiled outputs become
   editable sources without losing the original files.
3. Confirm the source files were created:
   ```
   ls src/skills/
   ```
4. If `skillet.toml` already exists but `.pan` source files are missing, run
   adoption manually: copy each `skills/<name>/SKILL.md` to
   `src/skills/<name>/<name>.pan` then add the required YAML frontmatter:
   ```markdown
   ---
   name: <name>
   description: "One-line description."
   ---
   ```
---

## Phase 2 — Resolve untyped references

Run lint in pedantic mode to surface all backtick spans that look like refs:

```
skillet lint --pedantic
```

For each `untyped-backtick` diagnostic, rewrite the backtick to the suggested
typed ref using the table below.  The lint output already tells you which type
to use (e.g. `looks like a path — consider \``ref`         | `./scripts/deploy.sh` `   | `ref::./scripts/deploy.sh`cmd`         | `kubectl apply -f …` `    | `cmd::kubectl`skill`       | `diagnose` `              | `skill::diagnose`url`         | `https://example.com` `   | `url::https://example.com`var`         | repeated project-specific string | extract to [vars]` and use `var::name` |
| `env`         | environment variable name     | `env::VAR_NAME`             |

After each batch of rewrites, re-run lint to confirm the warning count drops:

```
skillet lint --pedantic
```

Also scan for standard markdown links whose target is a local path:

```
[text](./path/to/file.md)
```

Where the link target should be treated as a file reference, inline it as a
typed ref: `ref::./path/to/file.md`.

---

## Phase 3 — Fragment extraction

Scan all `.pan` files for duplicate or near-duplicate passages that appear in
two or more skills.  Common candidates:

- Repeated safety / disclaimer notes
- Shared prerequisites or environment-setup sections
- Copy-pasted command-reference tables
- Boilerplate headers that vary only in the skill name

**Detection approach**

1. Open each `.pan` file and read the body (below the frontmatter).
2. Break each body into logical paragraphs or sections.
3. Compare paragraphs across files — flag any passage of ≥ 3 lines that
   appears verbatim or near-verbatim in two or more skills.
4. Also check `skillet lint` output for `duplication` diagnostics, which
   surface text that exceeds the similarity threshold automatically.

**Extraction steps for each candidate**

1. Create the fragment file:
   ```
   src/skills/_fragments/<name>.fragment.pan
   ```
   Paste the shared passage as its entire content (no frontmatter required).
2. Replace every occurrence in the `.pan` files with the Handlebars include:
   ```
   {