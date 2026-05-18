# Story: Scaffold a New Skill

## As a
Skill author working in an existing skillet workspace

## I want to
Run `skillet new <name>` to scaffold a new skill with minimal boilerplate

## So that
I can start writing skill instructions immediately with correct structure

## Acceptance Criteria

- [ ] `skillet new <name>` creates `skills/<name>/` directory
- [ ] Creates `skills/<name>/<name>.skill` with YAML frontmatter (`name`, empty `description`) and a heading
- [ ] Skill name in frontmatter matches the directory name
- [ ] Running `skillet new` with a name that already exists produces an error
- [ ] The scaffolded `.skill` file passes `skillet lint` except for the empty description warning
