//! Skillet — workspace scaffolding for skill-based AI agent projects.
//!
//! This crate provides the core logic for the `skillet` CLI tool.

pub mod budget;
pub mod check;
pub mod compile;
pub mod config;
pub mod lint;
pub mod lockfile;
pub mod net;
pub mod parse;
pub mod refs;
pub mod skill;
pub mod tokens;
pub mod workspace;

/// Backward-compat alias — prefer `compile`.
#[doc(hidden)]
pub use compile as build;
