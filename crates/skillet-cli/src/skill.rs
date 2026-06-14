//! `skillet skill` — print a bundled skill's compiled content to stdout.

use anyhow::{bail, Result};

static BUNDLED_SKILLS: &[(&str, &str)] = &[
    ("skillet", include_str!("../../../skills/skillet/SKILL.md")),
    ("migrate", include_str!("../../../skills/migrate/SKILL.md")),
    (
        "skillet-shim",
        include_str!("../../../skills/skillet-shim/SKILL.md"),
    ),
];

/// Lists all available bundled skill names to stdout.
pub fn list() {
    for (name, _) in BUNDLED_SKILLS {
        println!("{name}");
    }
}

/// Prints the compiled content of the named bundled skill to stdout.
///
/// # Errors
///
/// Returns an error if no bundled skill with the given name exists.
pub fn run(name: &str) -> Result<()> {
    match BUNDLED_SKILLS.iter().find(|(n, _)| *n == name) {
        Some((_, content)) => {
            print!("{}", content);
            Ok(())
        }
        None => {
            let available: Vec<&str> = BUNDLED_SKILLS.iter().map(|(n, _)| *n).collect();
            bail!(
                "no bundled skill named '{}'; available: {}",
                name,
                available.join(", ")
            )
        }
    }
}
