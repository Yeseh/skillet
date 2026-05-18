# Story: Fragment Reuse

## As a
Skill author with repeated instructions across multiple skills

## I want to
Extract shared content into fragments and include them in skill sources

## So that
I maintain a single source of truth and eliminate drift between skills

## Acceptance Criteria

- [ ] Fragment files are named `{name}.fragment.skill` and live in the workspace `_fragments/` directory
- [ ] Fragments are plain markdown content (no frontmatter required)
- [ ] Include syntax: `{{> fragment-name }}` on its own line (block-level only)
- [ ] Fragment content is inlined verbatim at the include site during build
- [ ] Multiple skills can include the same fragment
- [ ] A skill can include multiple different fragments
- [ ] Fragment includes cannot be nested (a fragment cannot include another fragment) — or can they?
- [ ] Missing fragment → build error
- [ ] Fragment token cost is reported in `skillet budget` per-skill
- [ ] `unused-fragment` lint warns about fragments no skill includes
- [ ] `oversized-fragment` lint warns when a fragment exceeds `max_fragment_tokens`
- [ ] Lockfile tracks fragment hashes and which skills use each fragment

## Open Question

- Should fragments support nesting (fragment includes another fragment)? Deferred for v1 — keep it flat.
