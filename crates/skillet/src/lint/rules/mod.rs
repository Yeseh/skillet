// Re-export shared lint types so rule files can use `use super::{...}`.
pub use super::{diag, diag_located, Diagnostic, Severity};

pub mod duplication;
pub mod invalid_frontmatter;
pub mod oversized;
pub mod stale_build;
pub mod stale_refs;
pub mod untyped_backtick;
pub mod unused_fragment;
