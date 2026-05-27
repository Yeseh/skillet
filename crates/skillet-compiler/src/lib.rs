mod lex;
pub mod parse;

pub struct SourceLocation {
    /// Line number (1 based)
    pub line: u32,
    /// Column number (1 based)
    pub column: u32,
}

#[non_exhaustive]
/// Represents the source content of a `.pan` file, along with its file path for error reporting.
pub struct PanSource {
    /// Absolute path to the source `.pan` file (for error reporting).
    pub path: Option<std::path::PathBuf>,
    /// Original source content of the `.pan` file.
    pub src: Box<str>,
    /// Line offset table
    /// Offsets are **byte** offsets into the source, not character indexes
    pub offsets: Vec<u32>,
}

impl PanSource {
    /// Creates a new [`PanSource`] from the given source string, without an associated file path.
    pub fn new(src: String, path: Option<std::path::PathBuf>) -> Self {
        let mut offsets: Vec<u32> = vec![0];
        let mut found_line_starts: Vec<u32> = src
            .char_indices()
            .filter(|c| c.1 == '\n')
            .map(|f| u32::try_from(f.0 + 1).expect("source offset exceeds u32"))
            .collect();

        offsets.append(found_line_starts.as_mut());

        Self {
            path,
            src: src.into_boxed_str(),
            offsets,
        }
    }

    /// Creates a new [`PanSource`] by reading the content from the specified file path.
    pub fn from_path(path: &std::path::Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;

        Ok(Self::new(content, Some(path.to_path_buf())))
    }

    pub fn as_str(&self) -> &str {
        &self.src
    }

    pub fn location_at(&self, offset: u32) -> SourceLocation {
        let line_idx = u32::try_from(self.offsets.partition_point(|&start| start <= offset))
            .expect("line index exceeds u32");
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
    fn test_single_line() {
        let ps = PanSource::new("Hello World".to_string(), None);

        assert_eq!(Some(0), ps.offsets.first().copied());
        assert_eq!(ps.offsets.len(), 1);
    }

    #[test]
    fn test_multi_line() {
        let ps = PanSource::new("Hello\nWorld".to_string(), None);

        let mut iter = ps.offsets.iter();
        assert_eq!(Some(0), iter.next().copied());
        assert_eq!(Some(6), iter.next().copied());
    }

    #[test]
    fn test_location_at() {
        let ps = PanSource::new("Hello\nWorld".to_string(), None);

        let loc = ps.location_at(9);
        assert_eq!(2, loc.line);
        assert_eq!(4, loc.column);
    }

    #[test]
    fn test_location_line_boundrary() {
        let ps = PanSource::new("Hello\nWorld".to_string(), None);

        let loc = ps.location_at(5);
        assert_eq!(1, loc.line);
        assert_eq!(6, loc.column);
    }
}
