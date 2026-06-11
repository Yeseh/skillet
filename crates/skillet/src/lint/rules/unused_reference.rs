//! Rule: `unused-reference` — warns when a reference file is not pointed to by any `ref::` in its skill.

use std::collections::HashSet;

use crate::refs::RefKind;
use crate::workspace::Workspace;

use super::{diag, CompiledSkill, Diagnostic, Severity};

/// Finds reference files that no `ref::` directive in the owning skill points to.
///
/// Usage is determined by scanning `cs.parsed_refs.typed` for `RefKind::Ref`
/// entries and matching their values against each reference's `relative_path`.
pub fn check(compiled: &[CompiledSkill], ws: &Workspace) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    for cs in compiled {
        let Some(skill) = ws.skills.get(&cs.name) else {
            continue;
        };

        if skill.references.is_empty() {
            continue;
        }

        let used: HashSet<&str> = cs
            .parsed_refs
            .typed
            .iter()
            .filter(|r| r.kind == RefKind::Ref)
            .map(|r| r.value.as_str())
            .collect();

        for reference in &skill.references {
            if !used.contains(reference.relative_path.as_str()) {
                diags.push(diag(
                    Severity::Warning,
                    &cs.name,
                    "unused-reference",
                    format!(
                        "reference '{}' is not pointed to by any ref:: in this skill",
                        reference.relative_path
                    ),
                ));
            }
        }
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compiler::compile::CompileOutput,
        refs::ParsedRefs,
        workspace::{skill::Reference, Skill},
    };
    use std::{collections::HashMap, path::PathBuf};

    fn make_compiled(name: &str, raw: &str) -> CompiledSkill {
        let skill_names: Vec<&str> = vec![];
        CompiledSkill {
            name: name.to_string(),
            module: "default".to_string(),
            source_path: PathBuf::from(format!("src/skills/{name}/{name}.pan")),
            raw: raw.to_string(),
            check_diags: vec![],
            output: CompileOutput {
                text: raw.to_string(),
                fragments_used: vec![],
                activation_tokens: 0,
                discovery_tokens: 0,
            },
            parsed_refs: ParsedRefs::extract(raw, &skill_names),
        }
    }

    fn make_workspace_with_skill(skill: Skill) -> Workspace {
        let mut skills = HashMap::new();
        skills.insert(skill.name.clone(), skill);
        Workspace {
            root: PathBuf::from("/tmp"),
            skills,
            agents: HashMap::new(),
            raw_fragments: HashMap::new(),
            fragments: crate::compiler::compile::RenderedFragments {
                rendered: HashMap::new(),
                poisoned: HashMap::new(),
            },
            fragment_hashes: HashMap::new(),
            fragment_tokens: HashMap::new(),
            fragment_paths: HashMap::new(),
            global_fragment_names: Default::default(),
            module_fragment_names: Default::default(),
            vars: Default::default(),
            env: Default::default(),
            allowed_commands: Default::default(),
            tokenizer: "cl100k_base".to_string(),
        }
    }

    fn make_skill(name: &str, references: Vec<Reference>) -> Skill {
        Skill {
            name: name.to_string(),
            module: "default".to_string(),
            source_path: PathBuf::from(format!("src/skills/{name}/{name}.pan")),
            src_dir: PathBuf::from(format!("src/skills/{name}")),
            target_dir: PathBuf::from(format!("dist/{name}")),
            scripts: vec![],
            references,
        }
    }

    fn make_reference(relative_path: &str) -> Reference {
        let name = relative_path
            .trim_start_matches("references/")
            .trim_end_matches(".pan")
            .to_string();
        Reference {
            name,
            relative_path: relative_path.to_string(),
            absolute_path: PathBuf::from(relative_path),
        }
    }

    #[test]
    fn no_diagnostics_when_no_references() {
        let skill = make_skill("diagnose", vec![]);
        let ws = make_workspace_with_skill(skill);
        let compiled = make_compiled("diagnose", "---\nname: diagnose\n---\n");
        assert!(check(&[compiled], &ws).is_empty());
    }

    #[test]
    fn no_diagnostics_when_all_references_used() {
        let reference = make_reference("references/api/types.pan");
        let skill = make_skill("diagnose", vec![reference]);
        let ws = make_workspace_with_skill(skill);
        let compiled = make_compiled(
            "diagnose",
            "---\nname: diagnose\n---\n\nSee `ref::references/api/types.pan`.\n",
        );
        assert!(check(&[compiled], &ws).is_empty());
    }

    #[test]
    fn warns_when_reference_not_used() {
        let reference = make_reference("references/api/types.pan");
        let skill = make_skill("diagnose", vec![reference]);
        let ws = make_workspace_with_skill(skill);
        let compiled = make_compiled("diagnose", "---\nname: diagnose\n---\n\nNo refs here.\n");
        let diags = check(&[compiled], &ws);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "unused-reference");
        assert_eq!(diags[0].skill, "diagnose");
        assert!(diags[0].message.contains("references/api/types.pan"));
    }

    #[test]
    fn warns_only_for_unused_references_not_all() {
        let used_ref = make_reference("references/api/types.pan");
        let unused_ref = make_reference("references/api/errors.pan");
        let skill = make_skill("diagnose", vec![used_ref, unused_ref]);
        let ws = make_workspace_with_skill(skill);
        let compiled = make_compiled(
            "diagnose",
            "---\nname: diagnose\n---\n\nSee `ref::references/api/types.pan`.\n",
        );
        let diags = check(&[compiled], &ws);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("references/api/errors.pan"));
    }

    #[test]
    fn skips_skill_not_in_workspace() {
        let ws = make_workspace_with_skill(make_skill("other", vec![]));
        let compiled = make_compiled("diagnose", "---\nname: diagnose\n---\n");
        assert!(check(&[compiled], &ws).is_empty());
    }
}
