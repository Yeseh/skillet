//! Skillet — workspace scaffolding for skill-based AI agent projects.
//!
//! This crate provides the core logic for the `skillet` CLI tool.

pub mod compile;
pub mod compiler;
pub mod config;
pub mod lint;
pub mod lockfile;
pub mod net;
pub mod parse;
pub mod refs;
pub mod skill;
pub mod tokens;
pub mod workspace;

#[cfg(test)]
pub(crate) mod test_support;
