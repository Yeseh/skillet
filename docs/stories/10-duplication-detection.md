# Story: Duplication Detection

## As a
Skill author maintaining multiple skills with overlapping instructions

## I want to
Be warned when near-verbatim content appears across multiple skills

## So that
I can extract it into a fragment and maintain a single source of truth

## Acceptance Criteria

- [ ] Detection uses near-verbatim matching: normalize whitespace and case, find shared n-grams
- [ ] Threshold: 3+ sentence sequences with >80% overlap trigger the lint
- [ ] `duplication` lint is severity: warning
- [ ] Report identifies which skills share the content and shows the duplicated passage
- [ ] Suggestion in output: "consider extracting to a fragment"
- [ ] Detection runs across compiled SKILL.md files (not sources, since fragments should already eliminate source-level duplication)
- [ ] Only cross-skill duplication is flagged (same content in a single skill is fine)
- [ ] `--format json` includes the duplicated text and affected skill names
