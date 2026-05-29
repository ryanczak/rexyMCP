//! Token count estimation. Heuristic — NOT a real BPE tokenizer.
//! Accuracy bar: within ~20% of real provider counts for typical
//! messages. The heuristic walks the input character by character,
//! grouping word-runs into ~4-char chunks (English text rate),
//! treating CJK characters as one token each, and counting most
//! ASCII punctuation as one token each.

/// Count the approximate number of tokens in a UTF-8 string.
pub fn count(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut word_chars = 0usize;

    for c in text.chars() {
        if c.is_whitespace() {
            tokens += word_to_tokens(word_chars);
            word_chars = 0;
        } else if is_cjk(c) || c.is_ascii_punctuation() {
            tokens += word_to_tokens(word_chars);
            word_chars = 0;
            tokens += 1;
        } else {
            word_chars += 1;
        }
    }
    tokens += word_to_tokens(word_chars);
    tokens
}

fn word_to_tokens(chars: usize) -> usize {
    if chars == 0 { 0 } else { chars.div_ceil(4) }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F |
        0x3040..=0x309F |
        0x30A0..=0x30FF |
        0x4E00..=0x9FFF |
        0xF900..=0xFAFF |
        0xAC00..=0xD7AF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_zero() {
        assert_eq!(count(""), 0);
    }

    #[test]
    fn single_short_word_is_one_token() {
        assert_eq!(count("hi"), 1);
    }

    #[test]
    fn long_word_proportional() {
        assert_eq!(count("abcdefghijklmnop"), 4);
    }

    #[test]
    fn punctuation_counts_individually() {
        let result = count("hi, world!");
        assert_eq!(result, 5);
    }

    #[test]
    fn cjk_each_char_is_one_token() {
        assert_eq!(count("你好世界"), 4);
    }

    #[test]
    fn mixed_content_within_accuracy_bar() {
        let text = "The quick brown fox jumps over the lazy dog. This is a typical English sentence with common words and punctuation! It should produce a reasonable token estimate.";
        let result = count(text);
        assert!(
            (30..=60).contains(&result),
            "expected 30-60 tokens for typical English sentence, got {result}"
        );
    }
}
