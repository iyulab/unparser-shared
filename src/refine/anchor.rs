//! Section anchor assignment.
//!
//! An event-level pass, same shape as [`crate::refine::link`] — an
//! event-level pass over the parsed stream, not a text-level one.
//! Assigns each heading a GitHub-compatible `id` via CommonMark's own
//! heading-attributes extension (`# text {#id}` — [kramdown]/Pandoc's
//! header-attribute syntax, which `pulldown-cmark` parses back into
//! `Tag::Heading::id` when [`Options::ENABLE_HEADING_ATTRIBUTES`] is on,
//! and `pulldown-cmark-to-cmark` already knows how to serialize). Chosen
//! over inserting a raw `<a id="...">` line: it's the CommonMark-ecosystem-
//! standard way to say "this heading has this id" rather than an invented
//! shape, and it round-trips through the parser itself, not just through
//! text — a heading that already carries an `id` (this pass's own earlier
//! output, or one a `un*` renderer or a human already set) is left alone,
//! which is what makes a second [`crate::refine::refine`] pass a no-op.
//!
//! The slug algorithm mirrors GitHub's own (documented upstream as the
//! `html-pipeline` Ruby filter's `[^\w -]` character class): lowercase,
//! drop everything that is not a Unicode letter/digit, `_`, `-`, or space,
//! then turn every space into a hyphen. Duplicate headings within one
//! document get GitHub's own disambiguation suffix (`-1`, `-2`, ...).
//!
//! [kramdown]: https://kramdown.gettalong.org/syntax.html#specifying-a-header-id
//! [`Options::ENABLE_HEADING_ATTRIBUTES`]: pulldown_cmark::Options::ENABLE_HEADING_ATTRIBUTES

use pulldown_cmark::{CowStr, Event, Tag, TagEnd};
use std::collections::HashMap;

/// Assigns a slug `id` to every heading in `events` that doesn't already
/// have one, using [`unique_slug`] (with a document-wide dedup table) to
/// derive it from the heading's own text.
pub(crate) fn assign_section_anchors(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    let mut out = Vec::with_capacity(events.len());
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut events = events.into_iter();

    while let Some(event) = events.next() {
        let Event::Start(Tag::Heading {
            level,
            id: None,
            classes,
            attrs,
        }) = event
        else {
            out.push(event);
            continue;
        };

        let mut inner = Vec::new();
        let mut text = String::new();
        for event in events.by_ref() {
            if matches!(&event, Event::End(TagEnd::Heading(_))) {
                break;
            }
            collect_text(&event, &mut text);
            inner.push(event);
        }

        let slug = unique_slug(&text, &mut seen);
        out.push(Event::Start(Tag::Heading {
            level,
            id: Some(CowStr::from(slug)),
            classes,
            attrs,
        }));
        out.extend(inner);
        out.push(Event::End(TagEnd::Heading(level)));
    }

    out
}

fn collect_text(event: &Event, out: &mut String) {
    match event {
        Event::Text(t) | Event::Code(t) => out.push_str(t),
        _ => {}
    }
}

/// A slug already used once in this document gets GitHub's own
/// disambiguation suffix (`-1`, `-2`, ...) on each further repeat. An empty
/// slug (a heading with no letters/digits at all, e.g. `# ---`) falls back
/// to `section` rather than emitting an empty `{#}`.
fn unique_slug(heading_text: &str, seen: &mut HashMap<String, u32>) -> String {
    let base = github_slug(heading_text);
    let base = if base.is_empty() {
        "section".to_string()
    } else {
        base
    };
    match seen.get_mut(&base) {
        None => {
            seen.insert(base.clone(), 0);
            base
        }
        Some(count) => {
            *count += 1;
            format!("{base}-{count}")
        }
    }
}

fn github_slug(text: &str) -> String {
    let lowered = text.to_lowercase();
    let kept: String = lowered
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == ' ')
        .collect();
    kept.replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refine::roundtrip::{parse, serialize};

    fn refine_events(input: &str) -> String {
        let events = assign_section_anchors(parse(input));
        serialize(events).unwrap()
    }

    #[test]
    fn assigns_a_slug_to_a_plain_heading() {
        assert_eq!(refine_events("# Overview\n"), "# Overview { #overview }");
    }

    #[test]
    fn lowercases_and_hyphenates_a_multi_word_heading() {
        assert_eq!(
            refine_events("## Getting Started\n"),
            "## Getting Started { #getting-started }"
        );
    }

    #[test]
    fn preserves_non_ascii_letters_in_the_slug() {
        assert_eq!(refine_events("# 한글 제목\n"), "# 한글 제목 { #한글-제목 }");
    }

    #[test]
    fn drops_punctuation_the_bare_slug_form_forbids() {
        assert_eq!(
            refine_events("# What's New?\n"),
            "# What's New? { #whats-new }"
        );
    }

    #[test]
    fn disambiguates_repeated_headings() {
        let output = refine_events("# Notes\n\ntext\n\n# Notes\n");
        assert!(output.contains("# Notes { #notes }"));
        assert!(output.contains("# Notes { #notes-1 }"));
    }

    #[test]
    fn leaves_a_heading_that_already_has_an_id_alone() {
        let input = "# Custom {#my-id}\n";
        assert_eq!(refine_events(input), "# Custom { #my-id }");
    }

    #[test]
    fn a_second_pass_is_a_no_op() {
        let once = refine_events("# Overview\n\n## Overview\n");
        let twice_events = assign_section_anchors(parse(&once));
        let twice = serialize(twice_events).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn leaves_non_heading_text_untouched() {
        let output = refine_events("just some text\n");
        assert_eq!(output, "just some text");
    }
}
