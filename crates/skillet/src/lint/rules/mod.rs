// Re-export shared lint types so rule files can use `use super::{...}`.
pub(super) use super::{diag, diag_located, Diagnostic, Severity};

pub(super) mod duplication;
pub(super) mod invalid_frontmatter;
pub(super) mod markdown_links;
pub(super) mod oversized;
pub(super) mod stale_build;
pub(super) mod stale_refs;
pub(super) mod untyped_backtick;
pub(super) mod unused_fragment;
