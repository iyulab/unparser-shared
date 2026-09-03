//! This module refines the *shape* of already-rendered markdown without
//! deleting any of its visible text.
//!
//! It is not a general markdown formatter, and it is not `cleanup` (the
//! noise-removal stage each `un*` renderer already ships). `cleanup` deletes —
//! page-number lines, repeated headers/footers, mojibake. It never
//! deletes: it enforces two invariants across every pass it runs.
//!
//! - **Lossless** — every visible text token present before a pass is still
//!   present after it (a superset relation: a pass may *add* text, such as a
//!   recovered table separator row, but never remove any).
//! - **Idempotent** — `refine(refine(x)) == refine(x)`.
//!
//! Because of those two invariants, this module composes with a renderer's
//! existing `cleanup` stage in either order, and never changes output that
//! has already shipped to a consumer.
//!
//! `0.1.0` implements all five passes the design calls for:
//! **table shape normalization** (missing-separator recovery and
//! ragged-row widening), **ordered-list renumbering**
//! (the round trip's serializer configuration), **link/image path
//! normalization** (backslash-to-forward-slash, duplicate-slash collapse —
//! real URLs and protocol-relative references are left
//! untouched), **frontmatter normalization** (canonical `key: value`
//! spacing, and dropping an entry or list item that
//! exactly repeats an earlier one; key order is left as authored, since
//! nothing asked for it to be sorted), and **section anchors**
//! (a GitHub-compatible slug assigned via CommonMark's own heading-
//! attributes extension, `# text {#id}`; a heading that already carries an
//! `id` is left alone).
//! [`refine`] always runs the CommonMark round trip (parse with
//! `pulldown-cmark`, re-serialize with `pulldown-cmark-to-cmark`) every pass
//! runs inside, plus the frontmatter handling and parse-failure fallback
//! that round trip needs — even with all five passes still to land, that
//! round trip alone is not a no-op (see `roundtrip`'s ordered-list
//! renumbering, which changes output on *any* ordered list, not only a
//! malformed one).
//!
//! One thing this pass deliberately does **not** attempt: a link or image
//! destination containing a raw space is not valid CommonMark syntax at all
//! outside `<...>` — it never becomes a `Link`/`Image` event to normalize.
//! Probing found all three `un*` renderers emit destinations unescaped, so
//! this can happen on real input; the fix belongs at the emission site instead.

mod anchor;
mod frontmatter;
mod link;
mod options;
mod roundtrip;
mod table;

pub use options::RefineOptions;

/// Refines the shape of `markdown` without deleting any of its visible text.
///
/// Returns `markdown` unchanged if it fails to parse as CommonMark. In
/// practice `pulldown-cmark` accepts effectively any input as literal text
/// rather than erroring, so this is a defensive floor — the
/// CommonMark-preserved invariant — not an expected path.
pub fn refine(markdown: &str, options: &RefineOptions) -> String {
    let _ = options;
    let (frontmatter, content) = frontmatter::extract(markdown);
    let recovered = table::recover_missing_separators(content);
    let widened = table::widen_ragged_tables(&recovered);
    let events = anchor::assign_section_anchors(link::normalize_link_destinations(
        roundtrip::parse(&widened),
    ));
    let Some(refined_body) = roundtrip::serialize(events) else {
        return markdown.to_string();
    };
    match frontmatter {
        Some(fm) => format!(
            "{}\n{}",
            frontmatter::normalize(fm),
            refined_body.trim_start()
        ),
        None => refined_body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_plain_markdown() {
        // `pulldown-cmark-to-cmark` does not reproduce trailing-newline
        // whitespace byte-for-byte — that is expected: the lossless
        // invariant this crate guarantees is over *visible text tokens*
        // (see `roundtrip_properties.rs`), not raw bytes.
        let options = RefineOptions::default();
        assert_eq!(refine("hello world\n", &options), "hello world");
    }

    #[test]
    fn preserves_frontmatter_verbatim() {
        let options = RefineOptions::default();
        let input = "---\ntitle: x\n---\n\nhello\n";
        let output = refine(input, &options);
        assert!(
            output.starts_with("---\ntitle: x\n---\n"),
            "frontmatter not preserved verbatim: {output:?}"
        );
    }

    #[test]
    fn drops_an_exact_duplicate_frontmatter_entry() {
        let options = RefineOptions::default();
        let input = "---\ntitle: \"x\"\ntitle: \"x\"\n---\n\nhello\n";
        let output = refine(input, &options);
        assert_eq!(
            output.matches("title:").count(),
            1,
            "exact duplicate entry not deduped: {output:?}"
        );
    }

    #[test]
    fn keeps_a_same_key_entry_with_a_different_value() {
        let options = RefineOptions::default();
        let input = "---\ntitle: \"x\"\ntitle: \"y\"\n---\n\nhello\n";
        let output = refine(input, &options);
        assert!(
            output.contains("\"x\"") && output.contains("\"y\""),
            "a non-duplicate value was dropped: {output:?}"
        );
    }

    #[test]
    fn assigns_a_section_anchor_to_a_heading() {
        let options = RefineOptions::default();
        let output = refine("# Getting Started\n", &options);
        assert!(
            output.contains("{ #getting-started }"),
            "no anchor assigned: {output:?}"
        );
    }

    #[test]
    fn round_trips_a_table() {
        let options = RefineOptions::default();
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let output = refine(input, &options);
        for token in ["a", "b", "1", "2"] {
            assert!(output.contains(token), "missing {token:?} in {output:?}");
        }
    }

    #[test]
    fn recovers_a_table_missing_its_separator_row() {
        // Integration-level check that the text pre-pass (`table.rs`) and the
        // CommonMark round trip compose: without recovery this input parses
        // as a plain paragraph containing literal `|` characters, not a
        // table, so `pulldown-cmark-to-cmark` would re-escape those pipes
        // (`\|`) rather than render them as table syntax.
        let options = RefineOptions::default();
        let input = "| a | b |\n| 1 | 2 |\n";
        let output = refine(input, &options);
        assert!(
            !output.contains(r"\|"),
            "pipes were escaped, meaning the block was not recognized as a table: {output:?}"
        );
        for token in ["a", "b", "1", "2"] {
            assert!(output.contains(token), "missing {token:?} in {output:?}");
        }
    }

    #[test]
    fn preserves_a_row_wider_than_the_header_by_widening_it() {
        // Regression: `pulldown-cmark` drops any cell beyond the header's
        // column count while *parsing*, even for an already-well-formed
        // table — a plain round trip with no passes at all already lost
        // "hello" here (found empirically while building this pass).
        // `refine` must widen the header, not just leave the loss
        // unaddressed.
        let options = RefineOptions::default();
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 | hello |\n";
        let output = refine(input, &options);
        assert!(output.contains("hello"), "excess cell was lost: {output:?}");
    }

    #[test]
    fn pads_a_short_data_row_with_an_empty_cell() {
        // pulldown-cmark itself pads a short row during parsing (probed
        // empirically) — this pins that the round trip materializes the
        // pad as real table syntax rather than dropping it.
        let options = RefineOptions::default();
        let input = "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 |\n";
        let output = refine(input, &options);
        let data_row = output
            .lines()
            .find(|line| line.contains('1'))
            .expect("data row present");
        assert_eq!(
            data_row.matches('|').count(),
            4,
            "expected a padded 3-cell row (4 pipes): {data_row:?}"
        );
    }
}
