//! The CommonMark parse → re-serialize round trip every pass in this module
//! runs inside.
//!
//! [`serialize`] is isolated from [`crate::refine::refine`] so the CommonMark-preserved
//! fallback (`crate::refine::refine` returning its input unchanged) stays a documented,
//! deliberate invariant rather than untested dead code. In the current
//! `String`-backed implementation it is, empirically, unreachable:
//! `pulldown_cmark_to_cmark::cmark`'s only error is `fmt::Error`, and
//! `String`'s `fmt::Write` impl never returns one — confirmed by attempting to
//! trigger it with a hand-built, structurally invalid event stream (an `End`
//! with no matching `Start`), which still serializes successfully. The guard
//! stays because the design invariant ("parse failure → original text,
//! unconditionally") must hold regardless of *how* `serialize` is implemented
//! later — e.g. a sink that can fail (a `Write`r, not a `String`) would make
//! this branch live again without any caller-visible change.

use pulldown_cmark::{Event, Options, Parser};

fn parser_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Parses `content` as CommonMark and re-serializes it, unchanged.
///
/// Returns `None` if serialization fails, so the caller can fall back to the
/// untouched original.
///
/// **Not** ragged-table-safe on its own: `pulldown-cmark` silently drops any
/// table cell beyond its header's column count while *parsing* — before this
/// function, or anything downstream of it, ever sees a `Vec<Event>`. Keeping
/// every table's header at least as wide as its widest row is
/// [`crate::refine::table::recover_missing_separators`]'s contract, enforced on
/// `content` *before* it reaches this function — round trip alone offers no
/// protection here.
///
/// `#[cfg(test)]`: production code (`crate::refine::refine`) needs an event-level
/// pass ([`crate::refine::link`]) between [`parse`] and [`serialize`], so it calls
/// those directly rather than through this convenience wrapper. Kept for
/// this module's own tests, where no such pass is under test.
#[cfg(test)]
pub(crate) fn round_trip(content: &str) -> Option<String> {
    serialize(parse(content))
}

/// Parses `content` as CommonMark. Exposed separately from [`round_trip`] so
/// event-level passes (see [`crate::refine::link`]) can rewrite the stream between
/// parsing and [`serialize`].
pub(crate) fn parse(content: &str) -> Vec<Event<'_>> {
    Parser::new_ext(content, parser_options()).collect()
}

/// Serializer options for [`serialize`]. Only `increment_ordered_list_bullets`
/// is set — the rest is `pulldown-cmark-to-cmark`'s own defaults (same as its
/// plain `cmark()` function). Left on unconditionally, not gated by a pass
/// flag: with it off, the serializer's default behavior is to print the
/// *first* item's number for every item in the list (verified empirically —
/// `1. / 5. / 3.` round-trips to `1. / 1. / 1.`), which is CommonMark-legal
/// but not the shape any consumer wants, on *any* ordered list, not only a
/// malformed one. Turning it on both normalizes an out-of-order or
/// same-number list to CommonMark's own sequential rule and stops the
/// round trip from degrading an already-correct list — the second half is
/// why this belongs in `roundtrip`, unconditional, rather than as an
/// opt-in `table`-style pass of its own.
fn serializer_options() -> pulldown_cmark_to_cmark::Options<'static> {
    pulldown_cmark_to_cmark::Options {
        increment_ordered_list_bullets: true,
        ..Default::default()
    }
}

pub(crate) fn serialize(events: Vec<Event>) -> Option<String> {
    let mut output = String::new();
    pulldown_cmark_to_cmark::cmark_with_options(
        events.into_iter(),
        &mut output,
        serializer_options(),
    )
    .ok()?;
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::TagEnd;

    #[test]
    fn round_trip_preserves_plain_text() {
        assert_eq!(round_trip("hello world").unwrap(), "hello world");
    }

    #[test]
    fn round_trip_preserves_table_cells() {
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let output = round_trip(input).unwrap();
        for token in ["a", "b", "1", "2"] {
            assert!(output.contains(token), "missing {token:?} in {output:?}");
        }
    }

    #[test]
    fn serialize_tolerates_a_structurally_invalid_event_stream() {
        // Documents the empirical finding in this module's doc comment: even
        // a hand-built, unbalanced stream (an `End` with no matching `Start`)
        // does not make `cmark()` return `Err` when writing into a `String`.
        // `round_trip` never builds a stream like this itself (it always
        // comes from a real `Parser`) — this just pins the observed behavior
        // so a future dependency bump that changes it is caught here, not
        // discovered as a silent new fallback path in `crate::refine::refine`.
        let events = vec![Event::End(TagEnd::Emphasis)];
        assert!(serialize(events).is_some());
    }

    #[test]
    fn round_trip_of_empty_content_is_empty() {
        assert_eq!(round_trip("").unwrap(), "");
    }

    #[test]
    fn renumbers_an_out_of_order_ordered_list_sequentially() {
        let output = round_trip("1. a\n5. b\n3. c\n").unwrap();
        assert_eq!(output, "1. a\n2. b\n3. c");
    }

    #[test]
    fn renumbers_a_same_number_ordered_list_sequentially() {
        // Legal CommonMark, but not what any consumer wants to see, and not
        // what "normalize" should mean for a numbered list.
        let output = round_trip("1. a\n1. b\n1. c\n").unwrap();
        assert_eq!(output, "1. a\n2. b\n3. c");
    }

    #[test]
    fn preserves_an_already_sequential_lists_start_value() {
        let output = round_trip("5. a\n6. b\n7. c\n").unwrap();
        assert_eq!(output, "5. a\n6. b\n7. c");
    }

    #[test]
    fn numbers_a_nested_ordered_list_independently_of_its_parent() {
        let output = round_trip("1. a\n   1. x\n   3. y\n2. b\n").unwrap();
        assert_eq!(output, "1. a\n   1. x\n   2. y\n2. b");
    }
}
