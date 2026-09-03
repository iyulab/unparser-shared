//! Property-test harness for the `refine` module's two governing invariants —
//! losslessness and idempotence. Every future pass must keep these green;
//! that is what makes it safe to compose passes without re-litigating
//! correctness pass by pass.

use proptest::prelude::*;
use std::collections::HashSet;
use unparser_shared::refine::{refine, RefineOptions};

/// Extracts every visible *word* directly from the raw `markdown` string —
/// split on whitespace and markdown/table punctuation, **without** parsing
/// it through `pulldown-cmark` first. This is the yardstick the lossless
/// invariant is checked against: a pass may add words, but every word
/// present before it must still be present after.
///
/// Parsing first would blind this check in exactly the cases this crate
/// exists to fix. Two examples found empirically while building the table
/// pass: a separator-less table parses as one merged paragraph
/// text run, so re-segmenting it into per-cell tokens *looks* like content
/// motion, not loss, only if compared post-parse — and a ragged row wider
/// than its header is silently clipped **during parsing itself**, so a
/// post-parse "before" set already lacks the excess cell's words, making a
/// real loss invisible to a before/after comparison entirely. Reading words
/// off the raw string sidesteps both: it can't be blinded by the very
/// parser step whose behavior these passes are working around.
///
/// Purely numeric tokens (`5`, `12`, ...) are excluded. In this generator's
/// vocabulary a bare number only ever comes from an ordered-list marker
/// (`word()` never produces digit-only content) — exactly the token
/// `roundtrip::serializer_options`'s renumbering is *supposed* to change —
/// a structural position, not lost content. If a future
/// generator needs numeric *content*, embed it in a non-numeric token (e.g.
/// `"item42"`) so it isn't swept up by this exclusion.
///
/// Backslash-escapes are collapsed first (found via a real renderer
/// sample, not a generator): `pulldown-cmark-to-cmark` correctly
/// escapes literal punctuation that would otherwise be misread on a future
/// parse (`[text]` with no destination becomes `\[text\]`, a literal `<<`
/// becomes `\<\<`) — the content is intact, but splitting on the inserted
/// `\` fractures a token like `<<` or `제3호서식]` into pieces that no
/// longer match their un-escaped form in the "before" set, a false loss.
/// Applying the same collapse to both sides keeps the comparison about
/// content, not serialization-time escaping.
///
/// `{`/`}` joined the split set later: a heading-attribute block
/// (`{#id}`, `{ #id }`) is delimiter syntax the serializer is free to
/// respace on any pass (`pulldown-cmark-to-cmark` always emits `{ #id }`
/// with spaces, regardless of how the input was spaced) — without splitting
/// on the braces themselves, a token glued to one (`hello}`) stops matching
/// its respaced form (`hello`, `}`) and reads as a false loss.
fn visible_text_tokens(markdown: &str) -> HashSet<String> {
    unescape_backslashes(markdown)
        .split(|c: char| c.is_whitespace() || "|-:#*_`>\\/.){}".contains(c))
        .filter(|word| !word.is_empty() && !word.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

/// Collapses `\X` to `X` **only** when `X` is one of CommonMark's
/// backslash-escapable ASCII punctuation characters — matching the spec's
/// own escape rule exactly. A backslash before anything else (a letter, a
/// digit) is not an escape in CommonMark and must stay two characters: the
/// `link_or_image` generator's Windows-style paths (`hello\sub\img.png`)
/// rely on that backslash surviving as real path-separator content, not
/// being swallowed as if it had escaped the `s` or `i` that follows it.
const ESCAPABLE_PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

fn unescape_backslashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if ESCAPABLE_PUNCTUATION.contains(next) {
                    out.push(next);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// A single markdown-shaped word, drawn from a small vocabulary that
/// includes ASCII and CJK content — the mix these renderers actually
/// produce.
fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("hello".to_string()),
        Just("world".to_string()),
        Just("data".to_string()),
        Just("표".to_string()),
        Just("한글".to_string()),
        Just("문서".to_string()),
    ]
}

fn plain_line() -> impl Strategy<Value = String> {
    prop::collection::vec(word(), 1..5).prop_map(|words| words.join(" "))
}

fn heading_line() -> impl Strategy<Value = String> {
    (1u8..=6, plain_line())
        .prop_map(|(level, text)| format!("{} {text}", "#".repeat(level as usize)))
}

/// A heading that already carries a `{#id}` -- exercises `anchor`'s
/// leave-alone path at scale, not just the one hand-written unit
/// test for it. `word()` doubles as the id text since it's already a safe
/// slug shape (no punctuation to escape).
fn heading_with_existing_id_line() -> impl Strategy<Value = String> {
    (1u8..=6, plain_line(), word())
        .prop_map(|(level, text, id)| format!("{} {text} {{#{id}}}", "#".repeat(level as usize)))
}

fn list_line() -> impl Strategy<Value = String> {
    plain_line().prop_map(|text| format!("- {text}"))
}

/// Two or three ordered-list items with an out-of-order or repeated start
/// number — exercises `roundtrip::serializer_options`'s renumbering
/// at scale, not just the hand-written unit tests for it.
fn ordered_list_block() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(1u32..9, 2..4),
        prop::collection::vec(word(), 2..4),
    )
        .prop_map(|(numbers, words)| {
            numbers
                .iter()
                .zip(words.iter().cycle())
                .map(|(n, w)| format!("{n}. {w}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn table_row() -> impl Strategy<Value = String> {
    prop::collection::vec(word(), 2..4).prop_map(|cells| format!("| {} |", cells.join(" | ")))
}

/// Two consecutive same-shaped pipe rows with **no** delimiter row between
/// them — the exact case `table::recover_missing_separators` targets. Both
/// invariants must still hold once that pre-pass rewrites this.
fn headerless_table_pair() -> impl Strategy<Value = String> {
    prop::collection::vec(word(), 2..4).prop_map(|cells| {
        let row = cells.join(" | ");
        format!("| {row} |\n| {row} |")
    })
}

/// A fenced code block whose body is pipe-row-shaped — the false-positive
/// case for `recover_missing_separators`: it must never insert a separator
/// inside a fence, and the code content itself must round-trip unchanged.
fn fenced_code_with_pipes() -> impl Strategy<Value = String> {
    plain_line().prop_map(|text| format!("```\n| {text} | {text} |\n```"))
}

/// A well-formed table whose last body row has one extra cell — the ragged
/// case `pulldown-cmark` clips silently during parsing. Exercises
/// header/separator widening at scale, not just the one hand-written unit
/// test for it.
fn ragged_table() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(word(), 2..3),
        prop::collection::vec(word(), 2..3),
        word(),
    )
        .prop_map(|(header, row, extra)| {
            let header_row = header.join(" | ");
            let sep = vec!["---"; header.len()].join(" | ");
            let wide_row = format!("{} | {extra}", row.join(" | "));
            format!("| {header_row} |\n| {sep} |\n| {wide_row} |")
        })
}

/// A link or image whose destination uses backslashes and/or duplicate
/// slashes — exercises `link::normalize_link_destinations` at
/// scale. Real URLs (`https://...`) and protocol-relative refs (`//...`)
/// are included too, as the must-not-touch control group.
fn link_or_image() -> impl Strategy<Value = String> {
    prop_oneof![
        (word(), word(), word()).prop_map(|(text, a, b)| format!(r"[{text}]({a}\{b}.png)")),
        (word(), word(), word()).prop_map(|(text, a, b)| format!("[{text}]({a}//{b}.png)")),
        (word(), word()).prop_map(|(alt, a)| format!(r"![{alt}]({a}\sub\img.png)")),
        word().prop_map(|text| format!("[{text}](https://example.com//path)")),
        word().prop_map(|text| format!("[{text}](//example.com/path)")),
    ]
}

/// A YAML frontmatter block exercising `frontmatter::normalize`:
/// `dup_mode` covers no duplicate / an exact duplicate `title:` entry (must
/// be deduped, but `title`'s content still survives via the kept copy) /
/// a same-key-**different**-value duplicate (must NOT be deduped -- both
/// values are distinct visible content, dropping either would violate
/// losslessness). `tags:` exercises unhwp's one-level block-sequence shape,
/// with its own possible exact-duplicate item.
fn frontmatter_block() -> impl Strategy<Value = String> {
    (
        word(),
        word(),
        word(),
        0u8..3,
        prop::bool::ANY,
        prop::collection::vec(word(), 1..3),
        prop::bool::ANY,
    )
        .prop_map(
            |(title, title2, author, dup_mode, has_tags, tags, dup_tag)| {
                let mut lines = vec!["---".to_string(), format!("title: \"{title}\"")];
                match dup_mode {
                    1 => lines.push(format!("title: \"{title}\"")),
                    2 => lines.push(format!("title: \"{title2}\"")),
                    _ => {}
                }
                lines.push(format!("author: \"{author}\""));
                if has_tags {
                    lines.push("tags:".to_string());
                    for t in &tags {
                        lines.push(format!("  - \"{t}\""));
                    }
                    if dup_tag {
                        lines.push(format!("  - \"{}\"", tags[0]));
                    }
                }
                lines.push("---".to_string());
                lines.join("\n")
            },
        )
}

/// A document with an optional frontmatter block prepended -- frontmatter
/// normalization runs on a wholly separate code path from every body pass,
/// so it needs its own generator composed alongside `markdown_doc`, not
/// folded into `line()`.
fn document_with_optional_frontmatter() -> impl Strategy<Value = String> {
    (prop::option::of(frontmatter_block()), markdown_doc()).prop_map(|(fm, body)| match fm {
        Some(fm) => format!("{fm}\n{body}"),
        None => body,
    })
}

fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => plain_line(),
        2 => heading_line(),
        1 => heading_with_existing_id_line(),
        2 => list_line(),
        1 => ordered_list_block(),
        2 => table_row(),
        1 => headerless_table_pair(),
        1 => fenced_code_with_pipes(),
        1 => ragged_table(),
        1 => link_or_image(),
        1 => Just(String::new()),
    ]
}

/// A markdown document assembled from a bounded number of lines drawn from
/// the strategies above — biased toward realistic shapes (headings, list
/// items, table rows) rather than raw arbitrary Unicode, so failures point at
/// real formatting bugs instead of edge cases no renderer in this family
/// would ever produce.
fn markdown_doc() -> impl Strategy<Value = String> {
    prop::collection::vec(line(), 0..12).prop_map(|lines| lines.join("\n"))
}

proptest! {
    #[test]
    fn refine_is_idempotent(doc in document_with_optional_frontmatter()) {
        let options = RefineOptions::default();
        let once = refine(&doc, &options);
        let twice = refine(&once, &options);
        prop_assert_eq!(once, twice);
    }

    #[test]
    fn refine_preserves_visible_text(doc in document_with_optional_frontmatter()) {
        let options = RefineOptions::default();
        let refined = refine(&doc, &options);
        let before = visible_text_tokens(&doc);
        let after = visible_text_tokens(&refined);
        prop_assert!(
            before.is_subset(&after),
            "lost tokens: {:?}",
            before.difference(&after).collect::<Vec<_>>()
        );
    }
}
