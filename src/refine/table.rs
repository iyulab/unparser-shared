//! Table shape normalization.
//!
//! This module has two stages, run in order, with deliberately different
//! epistemics.
//!
//! **[`recover_missing_separators`] — heuristic, conservative.**
//! `pulldown-cmark`'s GFM table extension only recognizes a `|`-delimited
//! block as a table when the line right after the header is a delimiter row
//! (only `-`, `:`, `|`, and whitespace). Real-world extraction can drop that
//! row while every data row stays intact — without it, the whole block
//! parses as a plain paragraph, invisible to every later structural pass.
//! This is a text-level pre-pass because by the time a missing-separator
//! table reaches the parser, there is no `Table` event left to repair — the
//! fix has to happen on raw lines, using a line-based approximation of
//! whether a table can even start here (see the function's own docs for why
//! that's necessarily conservative, and where it draws the line).
//!
//! **[`widen_ragged_tables`] — exact, parser-driven.** A second, more severe
//! finding surfaced while building the first stage: `pulldown-cmark` doesn't
//! just fail to *recognize* a separator-less table — it silently **drops**
//! any cell beyond the header's column count while parsing an
//! already-well-formed one. A plain CommonMark round trip loses data on
//! *any* table with a ragged row wider than its header, with or without
//! this module's other stage. Fixing that by re-deriving `pulldown-cmark`'s
//! own recognition rules (which constructs a table can and cannot interrupt
//! — a heading, yes; a list item's lazy-continued paragraph, no) would mean
//! reimplementing CommonMark block parsing. Instead, [`widen_ragged_tables`]
//! runs the real parser first, over the [`recover_missing_separators`]
//! output, and reads back the exact byte ranges it recognized as tables via
//! [`pulldown_cmark::OffsetIter`] — the parser's own recognition decisions,
//! not a re-derivation of them. It then widens each ragged table's header
//! and separator line with empty cells to the widest row present, so no row
//! is ever "excess" relative to the header `pulldown-cmark` parses against —
//! matching the design's stated direction ("초과는 유지 + 헤더 확장", excess
//! columns are kept via header expansion, never dropped or merged). Empty
//! cells rather than repeated header text: an empty cell adds nothing
//! semantically false, while repeating text would fabricate duplicate
//! content a downstream RAG chunker would ingest as real.
//!
//! Short rows need none of this: `pulldown-cmark` already pads them with an
//! empty cell during parsing, which the round trip in [`crate::refine::roundtrip`]
//! materializes on its own.
//!
//! Left out, still: collapsing colspan-residual empty cells has no reliable
//! signal in plain markdown to tell "merged cell" apart from "genuinely
//! empty cell" — collapsing on a heuristic would risk deleting real content,
//! which the lossless invariant forbids. That stays a human decision (see
//! the cycle log).

use std::ops::Range;

/// Recovers a missing GFM table delimiter row.
///
/// Fires where two consecutive non-blank lines both look like pipe-delimited
/// rows with the same cell count and neither is already a delimiter row —
/// the shape of a table whose separator row was dropped — **and** the
/// candidate header line opens a new block (it is the first line of
/// `content`, or the line before it is blank, or the line before it is the
/// closing delimiter of a fenced code block). Lines inside a fence are
/// never touched, so pipes in code are left alone.
///
/// The block-boundary requirement exists because CommonMark's lazy
/// continuation rule lets a `|`-starting line keep extending a *preceding*
/// block (a list item's paragraph, say) without any indentation signaling
/// that — a line that is textually pipe-row-shaped is not always at a
/// position where a table is even syntactically possible, and a text-level
/// scanner has no reliable way to tell without reimplementing CommonMark's
/// block-interruption rules (some constructs, like a heading or a plain
/// paragraph, *can* be interrupted by a table with no blank line; a list
/// item's lazy continuation cannot — verified empirically, not assumed).
/// Requiring a boundary is deliberately conservative rather than exact: it
/// costs nothing against this pass's actual input, since every `un*`
/// renderer separates block-level output with a blank line, and a missed
/// recovery opportunity is far cheaper than a wrong one. Without the gate, a
/// synthesized separator inserted into lazily-continued text becomes new
/// literal content on the *next* pass, at a different line position each
/// time — non-idempotent, and caught by
/// `tests/roundtrip_properties.rs`'s `refine_is_idempotent` property.
///
/// Once a table's header (real or just-synthesized) is recognized, the
/// entire contiguous run of pipe-rows that follows is copied verbatim as
/// its body — header detection never re-runs on an interior row. This is
/// not an optimization either: two adjacent body rows of an
/// already-well-formed table have the exact same shape as "a header missing
/// its separator", so skipping this would insert a spurious separator into
/// the *middle* of a real table on a second pass — the same idempotence
/// failure mode as above, just from a different cause.
///
/// Known limitations (documented, not silently assumed away): escaped
/// pipes (`\|`) inside a cell are counted as delimiters rather than
/// literal characters, and indented (4-space) code blocks are not tracked
/// the way fenced ones are. Neither case is expected in `un*`-rendered
/// markdown, which is this pass's only intended input.
pub(crate) fn recover_missing_separators(content: &str) -> String {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;
    let mut prev_blank = true; // start of document counts as a block boundary
    let mut i = 0;

    while i < lines.len() {
        let raw = lines[i];
        let text = trim_terminator(raw);
        let at_boundary = prev_blank;
        prev_blank = text.trim().is_empty();

        if is_fence_delimiter(text) {
            let was_in_fence = in_fence;
            in_fence = !in_fence;
            out.push_str(raw);
            i += 1;
            if was_in_fence {
                // Just closed a fence: unlike a paragraph or list item, a
                // fence has no lazy-continuation hazard — whatever follows
                // always starts a fresh block, blank line or not.
                prev_blank = true;
            }
            continue;
        }

        if in_fence {
            out.push_str(raw);
            i += 1;
            continue;
        }

        let Some(header_cells) = pipe_row_cell_count(text) else {
            out.push_str(raw);
            i += 1;
            continue;
        };

        let next_line = lines.get(i + 1);
        let next_text = next_line.map(|l| trim_terminator(l));
        let next_matches = next_text.and_then(pipe_row_cell_count) == Some(header_cells);
        let next_is_delimiter = next_text.map(is_delimiter_row).unwrap_or(false);
        let next_has_escape = next_text.map(contains_escaped_pipe).unwrap_or(false);

        let has_real_separator = next_matches && next_is_delimiter;
        let can_synthesize = next_matches
            && !next_is_delimiter
            && at_boundary
            && header_cells >= 2
            && !is_delimiter_row(text)
            && !contains_escaped_pipe(text)
            && !next_has_escape;

        if !has_real_separator && !can_synthesize {
            // No recognizable table start here — copy just this one line
            // and let the next iteration re-evaluate independently.
            out.push_str(raw);
            i += 1;
            continue;
        }

        out.push_str(raw);
        i += 1;

        if has_real_separator {
            out.push_str(lines[i]);
            i += 1;
        } else {
            out.push_str(&synthetic_separator(header_cells, line_terminator(raw)));
        }

        consume_pipe_row_body(&lines, &mut i, &mut out);
        prev_blank = false; // the last line copied was a table row, never blank
    }

    out
}

/// Copies the contiguous run of pipe-row-shaped lines starting at `*i` into
/// `out` verbatim, advancing `*i` past them.
fn consume_pipe_row_body(lines: &[&str], i: &mut usize, out: &mut String) {
    while *i < lines.len() && pipe_row_cell_count(trim_terminator(lines[*i])).is_some() {
        out.push_str(lines[*i]);
        *i += 1;
    }
}

/// Widens the header and separator of every table `pulldown-cmark` actually
/// recognizes in `content` whose body contains a row wider than its header —
/// see this module's top doc comment for why this runs the real parser as
/// an oracle instead of re-deriving its recognition rules.
pub(crate) fn widen_ragged_tables(content: &str) -> String {
    let spans = find_ragged_tables(content);
    if spans.is_empty() {
        return content.to_string();
    }

    let mut out = content.to_string();
    // Apply back-to-front so earlier byte ranges (still expressed against
    // `content`) stay valid as later ones are spliced in.
    for span in spans.iter().rev() {
        let header_text = &content[span.header_range.clone()];
        let header_line = trim_terminator(header_text);
        let header_terminator = line_terminator(header_text);
        let widened_header = format!(
            "{}{header_terminator}",
            rebuild_row(header_line, span.max_cells, "")
        );

        let sep_start = span.header_range.end;
        let sep_end = content[sep_start..]
            .find('\n')
            .map(|p| sep_start + p + 1)
            .unwrap_or(content.len());
        let sep_text = &content[sep_start..sep_end];
        let sep_line = trim_terminator(sep_text);
        let sep_terminator = line_terminator(sep_text);
        let widened_sep = format!(
            "{}{sep_terminator}",
            rebuild_row(sep_line, span.max_cells, "---")
        );

        out.replace_range(
            span.header_range.start..sep_end,
            &format!("{widened_header}{widened_sep}"),
        );
    }

    out
}

struct RaggedTable {
    header_range: Range<usize>,
    max_cells: usize,
}

/// Parses `content` once and, for every table `pulldown-cmark` recognizes
/// (list-item lazy-continuation exclusion, paragraph/heading interruption,
/// and every other block-structure rule already applied — by the parser,
/// not re-derived here), returns the ones whose widest row exceeds their
/// header's column count, in source order.
///
/// Cell count comes from re-splitting each `TableHead`/`TableRow` event's own
/// source *range* with [`pipe_row_cell_count`], not from counting
/// `TableCell` events — deliberately. Probing confirmed `pulldown-cmark`
/// drops the excess cell from the *event stream itself* while parsing a
/// ragged row, so a row of `| 1 | 2 | hello |` against a 2-column header
/// emits only two `TableCell` events; counting them would see width 2, not
/// 3, and never detect the raggedness this function exists to catch. The
/// row's byte range is not truncated the same way, so the raw text there
/// still has all three cells to recount.
fn find_ragged_tables(content: &str) -> Vec<RaggedTable> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    let parser_options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;

    let mut spans = Vec::new();
    let mut header_cells = 0usize;
    let mut header_range: Option<Range<usize>> = None;
    let mut max_cells = 0usize;

    for (event, range) in Parser::new_ext(content, parser_options).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(alignments)) => {
                header_cells = alignments.len();
                max_cells = header_cells;
                header_range = None;
            }
            Event::Start(Tag::TableHead) => {
                header_range = Some(range.clone());
                let row_text = trim_terminator(&content[range]);
                max_cells = max_cells.max(pipe_row_cell_count(row_text).unwrap_or(0));
            }
            Event::Start(Tag::TableRow) => {
                let row_text = trim_terminator(&content[range]);
                max_cells = max_cells.max(pipe_row_cell_count(row_text).unwrap_or(0));
            }
            Event::End(TagEnd::Table) => {
                if let Some(header_range) = header_range.take() {
                    if max_cells > header_cells {
                        spans.push(RaggedTable {
                            header_range,
                            max_cells,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    spans
}

/// Rebuilds a pipe row with exactly `target_cells` cells: `text`'s own cells
/// verbatim, padded up to `target_cells` with `pad_cell` for any it's short
/// by.
fn rebuild_row(text: &str, target_cells: usize, pad_cell: &str) -> String {
    let mut cells: Vec<&str> = split_cells(text).unwrap_or_default();
    let mut rendered: Vec<&str> = Vec::with_capacity(target_cells);
    rendered.append(&mut cells);
    while rendered.len() < target_cells {
        rendered.push(pad_cell);
    }
    format!("| {} |", rendered.join(" | "))
}

fn trim_terminator(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

fn line_terminator(line: &str) -> &'static str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn is_fence_delimiter(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Splits `text` on **unescaped** `|` characters — a `\|` is literal cell
/// content, not a delimiter, matching CommonMark's own escaping rule. Byte
/// scanning for the ASCII markers `\` and `|` is safe over UTF-8 text: every
/// continuation byte of a multi-byte character is `>= 0x80`, so it can never
/// be mistaken for either.
fn split_unescaped_pipes(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut backslashes = 0;
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'\\' {
            backslashes += 1;
            continue;
        }
        if b == b'|' && backslashes % 2 == 0 {
            parts.push(&text[start..idx]);
            start = idx + 1;
        }
        backslashes = 0;
    }
    parts.push(&text[start..]);
    parts
}

/// Splits a line into pipe-delimited cells, stripping one optional
/// leading/trailing `|` (the GFM convention for "outer" pipes). `None` means
/// the line has no unescaped interior pipe at all, so it isn't
/// table-row-shaped.
fn split_cells(text: &str) -> Option<Vec<&str>> {
    let mut parts = split_unescaped_pipes(text.trim());
    if parts.len() < 2 {
        return None;
    }
    if parts.first() == Some(&"") {
        parts.remove(0);
    }
    if parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.into_iter().map(str::trim).collect())
}

/// Whether `text` contains a literal escaped pipe (`\|`). Real tabular data
/// extracted from a source document never contains this — it is exclusively
/// an artifact of a markdown *serializer* escaping a literal `|` character
/// inside prose. Its presence is a strong signal that a line is already-
/// serialized paragraph text, not raw table input, even when it happens to
/// still be pipe-row-shaped by cell count.
fn contains_escaped_pipe(text: &str) -> bool {
    text.contains("\\|")
}

fn pipe_row_cell_count(text: &str) -> Option<usize> {
    split_cells(text).map(|cells| cells.len())
}

fn is_delimiter_row(text: &str) -> bool {
    match split_cells(text) {
        Some(cells) if !cells.is_empty() => cells.iter().copied().all(is_delimiter_cell),
        _ => false,
    }
}

fn is_delimiter_cell(cell: &str) -> bool {
    let core = cell.strip_prefix(':').unwrap_or(cell);
    let core = core.strip_suffix(':').unwrap_or(core);
    !core.is_empty() && core.bytes().all(|b| b == b'-')
}

fn synthetic_separator(cell_count: usize, terminator: &str) -> String {
    let cells = vec!["---"; cell_count].join(" | ");
    format!("| {cells} |{terminator}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_separator_between_header_and_data_row() {
        let input = "| a | b |\n| 1 | 2 |\n";
        let output = recover_missing_separators(input);
        assert_eq!(output, "| a | b |\n| --- | --- |\n| 1 | 2 |\n");
    }

    #[test]
    fn leaves_a_well_formed_table_untouched() {
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn does_not_touch_pipes_inside_a_fenced_code_block() {
        let input = "```\n| not | a | table |\n| still | not |\n```\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn ignores_a_lone_pipe_row_with_no_matching_second_row() {
        let input = "| a | b |\nplain text\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn ignores_single_column_rows() {
        // header_cells == 1 is below the >=2 threshold — too weak a signal
        // to distinguish from a stray pipe in prose.
        let input = "| a |\n| 1 |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn requires_matching_cell_counts_to_fire() {
        let input = "| a | b |\n| 1 | 2 | 3 |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn recovers_a_multi_row_table_body_in_one_pass() {
        let input = "| a | b |\n| 1 | 2 |\n| 3 | 4 |\n";
        let output = recover_missing_separators(input);
        assert_eq!(output, "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n");
    }

    #[test]
    fn preserves_crlf_line_endings_in_the_synthetic_row() {
        let input = "| a | b |\r\n| 1 | 2 |\r\n";
        let output = recover_missing_separators(input);
        assert_eq!(output, "| a | b |\r\n| --- | --- |\r\n| 1 | 2 |\r\n");
    }

    #[test]
    fn does_not_re_insert_a_separator_inside_an_already_well_formed_multi_row_table() {
        // Regression (a non-idempotence regression): two adjacent body
        // rows of a real table have the same shape as "header missing its
        // separator". Running recovery a second time on an already-correct
        // table must be a no-op.
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn is_idempotent_on_output_it_just_recovered() {
        let input = "| a | b |\n| 1 | 2 |\n| 3 | 4 |\n";
        let once = recover_missing_separators(input);
        let twice = recover_missing_separators(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn does_not_fire_on_pipe_rows_lazily_continuing_a_list_item() {
        // Regression (a non-idempotence regression): without a
        // preceding blank line, "| hello | hello |" right after a list item
        // is CommonMark lazy-continuation text for that item, not a table
        // start — inserting a separator here manufactures new literal
        // content on the *next* pass instead of recognizing a table.
        let input = "- hello\n| hello | hello |\n| hello | hello |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn fires_after_a_blank_line_following_other_content() {
        let input = "- hello\n\n| a | b |\n| 1 | 2 |\n";
        let output = recover_missing_separators(input);
        assert_eq!(output, "- hello\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n");
    }

    #[test]
    fn does_not_fire_on_lines_carrying_an_escaped_pipe() {
        // Regression (a non-idempotence regression): a heading
        // directly followed (no blank line) by headerless pipe-rows first
        // round-trips as an escaped-pipe paragraph, WITH a canonical blank
        // line the serializer inserts after the heading. That blank line
        // would satisfy the block-boundary gate on a *second* pass even
        // though this was never a table — the escaped-pipe signal is what
        // correctly refuses it regardless.
        let input = "# hello\n\n\\| hello | hello |\n\\| hello | hello |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn counts_cells_correctly_when_content_has_an_escaped_pipe() {
        // A real table whose cell content happens to contain a literal
        // escaped pipe must still be counted correctly.
        let input = "| a\\|b | c |\n| --- | --- |\n| 1 | 2 |\n";
        assert_eq!(recover_missing_separators(input), input);
    }

    #[test]
    fn widen_ragged_tables_expands_header_and_separator() {
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 | hello |\n";
        let output = widen_ragged_tables(input);
        assert_eq!(
            output,
            "| a | b |  |\n| --- | --- | --- |\n| 1 | 2 | hello |\n"
        );
    }

    #[test]
    fn widen_ragged_tables_leaves_non_ragged_tables_untouched() {
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        assert_eq!(widen_ragged_tables(input), input);
    }

    #[test]
    fn widen_ragged_tables_recognizes_a_table_interrupting_a_paragraph() {
        // pulldown-cmark lets a real table (header + separator) interrupt a
        // plain paragraph with no blank line — verified empirically. Uses
        // the parser as the oracle, so this needs no boundary gate at all.
        let input = "para\n| a | b |\n| --- | --- |\n| 1 | 2 | hello |\n";
        let output = widen_ragged_tables(input);
        assert!(output.contains("hello"), "excess cell lost: {output:?}");
    }

    #[test]
    fn widen_ragged_tables_leaves_a_list_items_lazy_text_alone() {
        // pulldown-cmark does NOT let a table interrupt a list item's lazy
        // continuation — verified empirically (it swallows the whole thing
        // as plain paragraph text, no Table event at all). Nothing to widen.
        let input = "- x\n| a | b |\n| --- | --- |\n| 1 | 2 | hello |\n";
        assert_eq!(widen_ragged_tables(input), input);
    }

    #[test]
    fn widen_ragged_tables_is_idempotent() {
        let input = "| a | b |\n| --- | --- |\n| 1 | 2 | hello |\n";
        let once = widen_ragged_tables(input);
        let twice = widen_ragged_tables(&once);
        assert_eq!(once, twice);
    }
}
