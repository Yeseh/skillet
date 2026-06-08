//! Internal parser/compiler support for `.pan` syntax.
//!
//! This module hosts the absorbed `skillet-compiler` crate as a nested module
//! within `skillet`, preserving its lexer/parser split while making the API
//! available as `skillet::compiler`.

pub mod compile;
pub mod lex;
pub mod parse;

use std::path::{Path};

use crate::workspace::artefact::Artefact;

/// A 1-based line/column location within a source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    /// Line number, starting at 1.
    pub line: u32,
    /// Column number, starting at 1.
    pub column: u32,
}

/// Source content of a `.pan` file plus metadata used for diagnostics.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PanSource {
    /// Original source content.
    pub src: Box<str>,
    /// Byte offsets of the first character of each line.
    pub offsets: Vec<u32>,
}

impl PanSource {
    /// Creates a new source wrapper from raw text and an optional path.
    #[must_use]
    pub fn new(src: String) -> Self {
        let mut offsets: Vec<u32> = vec![0];
        let mut found_line_starts: Vec<u32> = src
            .char_indices()
            .filter(|c| c.1 == '\n')
            .map(|f| (f.0 + 1) as u32)
            .collect();

        offsets.append(&mut found_line_starts);

        Self {
            src: src.into_boxed_str(),
            offsets,
        }
    }

    pub fn from_artefact(artefact: Artefact) -> std::io::Result<Self> {
        Self::from_path(&artefact.source_path)
    }

    /// Loads a source file from disk.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from reading the file.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::new(content))
    }

    /// Returns the original source text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// Converts a byte offset into a 1-based line/column location.
    #[must_use]
    pub fn location_at(&self, offset: u32) -> SourceLocation {
        let line_idx = self.offsets.partition_point(|&start| start <= offset) as u32;
        let line_offset = self.offsets[(line_idx - 1) as usize];
        let column = offset - line_offset + 1;

        SourceLocation {
            line: line_idx,
            column,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_keeps_only_zero_offset() {
        let ps = PanSource::new("Hello World".to_string());

        assert_eq!(Some(0), ps.offsets.first().copied());
        assert_eq!(ps.offsets.len(), 1);
    }

    #[test]
    fn multi_line_records_each_line_start() {
        let ps = PanSource::new("Hello\nWorld".to_string());

        let mut iter = ps.offsets.iter();
        assert_eq!(Some(0), iter.next().copied());
        assert_eq!(Some(6), iter.next().copied());
    }

    #[test]
    fn location_at_reports_line_and_column() {
        let ps = PanSource::new("Hello\nWorld".to_string());

        let loc = ps.location_at(9);
        assert_eq!(2, loc.line);
        assert_eq!(4, loc.column);
    }

    #[test]
    fn location_at_handles_line_boundary() {
        let ps = PanSource::new("Hello\nWorld".to_string());

        let loc = ps.location_at(5);
        assert_eq!(1, loc.line);
        assert_eq!(6, loc.column);
    }
}
