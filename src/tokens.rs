//! Token-counting utilities backed by tiktoken-rs.

use tiktoken_rs::tokenizer::Tokenizer;
use tiktoken_rs::bpe_for_tokenizer;

/// Returns the number of tokens in `text` using the given tiktoken encoding name
/// (e.g. `"cl100k_base"`, `"o200k_base"`).
///
/// Falls back to the `⌈chars / 4⌉` heuristic when the name is not recognised
/// so that the rest of the tool keeps working even with unknown future encodings.
pub fn count_tokens(text: &str, tokenizer: &str) -> u32 {
    let tok = match tokenizer {
        "cl100k_base" => Tokenizer::Cl100kBase,
        "o200k_base" => Tokenizer::O200kBase,
        "o200k_harmony" => Tokenizer::O200kHarmony,
        "p50k_base" => Tokenizer::P50kBase,
        "p50k_edit" => Tokenizer::P50kEdit,
        "r50k_base" | "gpt2" => Tokenizer::R50kBase,
        _ => return approx_tokens(text),
    };
    match bpe_for_tokenizer(tok) {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u32,
        Err(_) => approx_tokens(text),
    }
}

/// Approximates token count as `⌈chars / 4⌉` (GPT tokeniser average for English).
///
/// Used as a fallback when the configured tokenizer name is not recognised.
pub(crate) fn approx_tokens(text: &str) -> u32 {
    text.chars().count().div_ceil(4) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cl100k_base_empty_string_is_zero_tokens() {
        assert_eq!(count_tokens("", "cl100k_base"), 0);
    }

    #[test]
    fn cl100k_base_counts_real_tokens() {
        // "hello world" is 2 tokens in cl100k_base
        assert_eq!(count_tokens("hello world", "cl100k_base"), 2);
    }

    #[test]
    fn o200k_base_counts_tokens() {
        assert!(count_tokens("hello world", "o200k_base") > 0);
    }

    #[test]
    fn unknown_tokenizer_falls_back_to_approx() {
        // approx: 11 chars / 4 = 3 (rounded up)
        assert_eq!(count_tokens("hello world", "unknown_enc"), 3);
    }

    #[test]
    fn fallback_empty_string_is_zero_tokens() {
        assert_eq!(count_tokens("", "unknown_enc"), 0);
    }
}
