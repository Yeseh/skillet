use lsp_types::{Position, Range};
use skillet::navigation::ByteRange;

/// Converts an LSP UTF-16 position to a UTF-8 byte offset.
pub fn lsp_position_to_byte_offset(text: &str, position: Position) -> Option<usize> {
    let mut line_start = 0;
    let mut line_number = 0;

    loop {
        let line_feed = text[line_start..]
            .find('\n')
            .map(|offset| line_start + offset);
        let line_end = line_feed.unwrap_or(text.len());
        let content_end = if line_feed.is_some() && line_end > line_start {
            if text.as_bytes()[line_end - 1] == b'\r' {
                line_end - 1
            } else {
                line_end
            }
        } else {
            line_end
        };

        if line_number == position.line {
            let target = usize::try_from(position.character).ok()?;
            let line = &text[line_start..content_end];
            let mut utf16_offset = 0;

            for (byte_offset, character) in line.char_indices() {
                if utf16_offset == target {
                    return Some(line_start + byte_offset);
                }

                let character_width = character.len_utf16();
                if utf16_offset + character_width > target {
                    return None;
                }
                utf16_offset += character_width;
            }

            return (utf16_offset == target).then_some(content_end);
        }

        line_start = line_feed? + 1;
        line_number += 1;
    }
}

/// Converts a UTF-8 byte offset to an LSP UTF-16 position.
pub fn byte_offset_to_lsp_position(text: &str, byte_offset: usize) -> Option<Position> {
    if byte_offset > text.len() || !text.is_char_boundary(byte_offset) {
        return None;
    }
    if byte_offset > 0
        && text.as_bytes().get(byte_offset - 1) == Some(&b'\r')
        && text.as_bytes().get(byte_offset) == Some(&b'\n')
    {
        return None;
    }

    let prefix = &text[..byte_offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |offset| offset + 1);
    let character = text[line_start..byte_offset].encode_utf16().count();

    Some(Position {
        line: u32::try_from(line).ok()?,
        character: u32::try_from(character).ok()?,
    })
}

/// Converts a byte range in source text to its LSP range.
pub fn byte_range_to_lsp_range(text: &str, range: ByteRange) -> Option<Range> {
    Some(Range {
        start: byte_offset_to_lsp_position(text, range.start)?,
        end: byte_offset_to_lsp_position(text, range.end)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_convert_between_utf16_and_utf8_with_crlf() {
        let text = "abc\r\né😀x\r\n";
        let cases = [
            (Position::new(0, 0), 0),
            (Position::new(0, 3), 3),
            (Position::new(1, 0), 5),
            (Position::new(1, 1), 7),
            (Position::new(1, 3), 11),
            (Position::new(1, 4), 12),
            (Position::new(2, 0), 14),
        ];

        for (position, offset) in cases {
            assert_eq!(lsp_position_to_byte_offset(text, position), Some(offset));
            assert_eq!(byte_offset_to_lsp_position(text, offset), Some(position));
        }
    }

    #[test]
    fn positions_reject_the_middle_of_a_surrogate_pair() {
        let text = "a😀b";

        assert_eq!(lsp_position_to_byte_offset(text, Position::new(0, 2)), None);
    }

    #[test]
    fn byte_offsets_between_crlf_bytes_are_not_lsp_positions() {
        let text = "line\r\nnext";

        assert_eq!(byte_offset_to_lsp_position(text, 5), None);
        assert_eq!(lsp_position_to_byte_offset(text, Position::new(0, 5)), None);
    }

    #[test]
    fn byte_ranges_preserve_multibyte_character_positions() {
        let text = "é😀 ref::target";
        let start = text.find("ref::").unwrap();
        let range = ByteRange {
            start,
            end: start + "ref::target".len(),
        };

        assert_eq!(
            byte_range_to_lsp_range(text, range),
            Some(Range::new(Position::new(0, 4), Position::new(0, 15)))
        );
    }
}
