//! Shared token-counting utilities.

/// Approximates token count as `⌈chars / 4⌉` (GPT tokeniser average for English).
///
/// This is intentionally a rough heuristic — the same one used by `skillet lint`
/// and `skillet budget` so counts stay consistent across commands.
pub fn approx_tokens(text: &str) -> u32 {
    text.len().div_ceil(4) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_zero_tokens() {
        assert_eq!(approx_tokens(""), 0);
    }

    #[test]
    fn four_chars_is_one_token() {
        assert_eq!(approx_tokens("abcd"), 1);
    }

    #[test]
    fn five_chars_rounds_up_to_two_tokens() {
        assert_eq!(approx_tokens("abcde"), 2);
    }
}
