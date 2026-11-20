use std::ops::Range;

pub(crate) fn word_range(
    chars: &[char],
    index: usize,
    is_word: impl Fn(char) -> bool,
) -> Option<Range<usize>> {
    if chars.is_empty() {
        return None;
    }
    let mut index = index.min(chars.len());
    if index == chars.len() {
        index -= 1;
    }
    let word = is_word(chars[index]);
    let mut start = index;
    while start > 0 && is_word(chars[start - 1]) == word {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && is_word(chars[end]) == word {
        end += 1;
    }
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::word_range;

    fn is_word(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    #[test]
    fn groups_neighboring_characters_of_the_same_class() {
        let chars: Vec<char> = "hello, world".chars().collect();
        assert_eq!(word_range(&chars, 1, is_word), Some(0..5));
        assert_eq!(word_range(&chars, 6, is_word), Some(5..7));
        assert_eq!(word_range(&chars, 8, is_word), Some(7..12));
        assert_eq!(word_range(&chars, 12, is_word), Some(7..12));
    }

    #[test]
    fn empty_input_has_no_word() {
        assert_eq!(word_range(&[], 0, is_word), None);
    }
}
