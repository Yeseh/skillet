// Re-export shared lint types so rule files can use `use super::{...}`.
pub(crate) use super::frontmatter_field_line;
pub use super::{diag, diag_located, CompiledSkill, Diagnostic, RelatedLocation, Severity};

pub mod duplication;
pub mod invalid_frontmatter;
pub mod oversized;
pub mod untyped_backtick;
pub mod unused_fragment;
pub mod unused_reference;
