//! CommonMark syntax facts every Markdown writer in the family needs.
//!
//! Each library renders its own document model, and how it styles a run is its own business.
//! *Where* an emphasis delimiter may stand is not — it is a property of CommonMark, and a
//! writer that gets it wrong prints its asterisks as text. This module answers that one
//! question so the three writers cannot drift on it.
//!
//! `std`-only, like the rest of the crate root.

use std::ops::Range;

/// The part of a styled run that emphasis delimiters (`*`, `**`, `~~`) can wrap, given the
/// characters the run lands between, or `None` when no part can.
///
/// `before` is the last character already written, `after` the first one the next piece will
/// write (`None` at either end of the inline content). The text outside the returned range is
/// written as-is, outside the delimiters.
///
/// CommonMark reads a delimiter as emphasis only when it *flanks* the text (§6.2): it may not
/// touch whitespace on its inside, and when its inside is punctuation its outside must be
/// whitespace, punctuation, or the end of the line. So:
///
/// - leading and trailing whitespace always stays outside — `**word **` is not emphasis;
/// - leading or trailing punctuation stays outside too, with any whitespace behind it, when
///   the run touches a letter or digit on that side — an italic `", s"` after `32` written as
///   `32*, s*` shows its asterisks, written as `32, *s*` it does not;
/// - a run that is nothing but whitespace, or nothing but punctuation between words, has
///   nothing to wrap.
///
/// Punctuation that does not touch a word stays inside (`*i.e.,*`, `*(h)*`), so a run is not
/// reshaped unless it has to be.
///
/// `text` may already be escaped: a backslash escape (`\*`) is one unit and is never split
/// across a delimiter. Only escapes of ASCII punctuation are units, as in CommonMark.
///
/// ```
/// use unparser_shared::markdown::emphasis_span;
///
/// let wrap = |text: &str, before, after| {
///     emphasis_span(text, before, after).map_or(text.to_string(), |r| {
///         format!("{}*{}*{}", &text[..r.start], &text[r.clone()], &text[r.end..])
///     })
/// };
/// assert_eq!(wrap(", s", Some('2'), Some(' ')), ", *s*");
/// assert_eq!(wrap("word ", None, Some('x')), "*word* ");
/// assert_eq!(wrap(",", Some('8'), Some('a')), ",");
/// assert_eq!(wrap("(h)", Some(' '), None), "*(h)*");
/// ```
pub fn emphasis_span(
    text: &str,
    before: Option<char>,
    after: Option<char>,
) -> Option<Range<usize>> {
    let mut start = text.len() - text.trim_start().len();
    let mut end = text.trim_end().len();
    if start >= end {
        return None;
    }
    let touches_a_word = |c: Option<char>| c.is_some_and(|c| !is_flanking_neutral(c));
    if start == 0 && touches_a_word(before) {
        start = units(text, start, end)
            .find(|&(i, j)| !unit_is_neutral(&text[i..j]))
            .map_or(end, |(i, _)| i);
    }
    if end == text.len() && touches_a_word(after) {
        end = units(text, start, end)
            .filter(|&(i, j)| !unit_is_neutral(&text[i..j]))
            .last()
            .map_or(start, |(_, j)| j);
    }
    (start < end).then_some(start..end)
}

/// Whitespace or punctuation — what a delimiter may touch on its outside when its inside is
/// punctuation. CommonMark's punctuation is the Unicode P and S categories; a character that
/// is neither alphanumeric, whitespace nor a control is one of them, save for combining
/// marks, which do not stand at a run's edge.
fn is_flanking_neutral(c: char) -> bool {
    c.is_whitespace() || !(c.is_alphanumeric() || c.is_control())
}

/// A unit is neutral when the character it writes is: an escape writes punctuation.
fn unit_is_neutral(unit: &str) -> bool {
    unit.chars().last().is_some_and(is_flanking_neutral)
}

/// The byte ranges of `text[start..end]`'s units: one character, or a backslash and the ASCII
/// punctuation it escapes.
fn units(text: &str, start: usize, end: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut chars = text[start..end].char_indices().peekable();
    std::iter::from_fn(move || {
        let (i, c) = chars.next()?;
        let mut j = i + c.len_utf8();
        if c == '\\' {
            if let Some(&(k, escaped)) = chars.peek() {
                if escaped.is_ascii_punctuation() {
                    j = k + escaped.len_utf8();
                    chars.next();
                }
            }
        }
        Some((start + i, start + j))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `*` around its emphasis span.
    fn wrap(text: &str, before: Option<char>, after: Option<char>) -> String {
        emphasis_span(text, before, after).map_or(text.to_string(), |r| {
            format!(
                "{}*{}*{}",
                &text[..r.start],
                &text[r.clone()],
                &text[r.end..]
            )
        })
    }

    #[test]
    fn whitespace_stays_outside() {
        assert_eq!(wrap("  word  ", Some('a'), Some('b')), "  *word*  ");
        assert_eq!(emphasis_span("   ", None, None), None);
        assert_eq!(emphasis_span("", None, None), None);
    }

    #[test]
    fn leading_punctuation_against_a_word_moves_out_with_its_space() {
        assert_eq!(wrap(", s", Some('2'), Some(' ')), ", *s*");
    }

    #[test]
    fn trailing_punctuation_against_a_word_moves_out() {
        assert_eq!(wrap("word.", Some(' '), Some('x')), "*word*.");
        assert_eq!(wrap("word, ", Some(' '), Some('x')), "*word,* ");
    }

    #[test]
    fn punctuation_between_words_alone_has_nothing_to_wrap() {
        assert_eq!(emphasis_span(",", Some('8'), Some('a')), None);
        assert_eq!(emphasis_span(", ", Some('8'), None), None);
    }

    #[test]
    fn punctuation_against_space_punctuation_or_an_edge_stays_inside() {
        assert_eq!(wrap("i.e.,", Some(' '), Some(' ')), "*i.e.,*");
        assert_eq!(wrap("(h)", None, None), "*(h)*");
        assert_eq!(wrap("(h)", Some('('), Some(')')), "*(h)*");
        assert_eq!(
            wrap("Continued pretraining.", None, Some(' ')),
            "*Continued pretraining.*"
        );
    }

    #[test]
    fn a_word_on_either_side_moves_both_ends() {
        assert_eq!(wrap("(h)", Some('Q'), Some('x')), "(*h*)");
    }

    #[test]
    fn an_escape_is_never_split() {
        assert_eq!(wrap(r"\*x\_", Some('a'), Some('b')), r"\**x*\_");
    }

    #[test]
    fn a_backslash_before_a_letter_is_not_an_escape() {
        assert_eq!(wrap(r"\a", Some('b'), None), r"\*a*");
    }

    #[test]
    fn non_ascii_punctuation_and_letters_count() {
        assert_eq!(wrap("“word”", Some('x'), Some('y')), "“*word*”");
        assert_eq!(wrap("단어", Some('가'), Some('나')), "*단어*");
    }
}
