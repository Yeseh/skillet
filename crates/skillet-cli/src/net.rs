//! Network I/O for the CLI — URL verification during build.
//!
//! Re-exports the implementation from the `skillet` lib crate.  The
//! implementation will migrate here fully in a later refactor step.

pub use skillet::net::url_verify::{verify_urls, UrlCheckOutcome, UrlCheckResult};
