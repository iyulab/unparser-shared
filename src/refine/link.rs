//! Link and image destination normalization.
//!
//! Unlike [`crate::refine::table`]'s missing-separator recovery, this is a plain
//! event-level pass, not a text-level pre-pass — deliberately, following
//! the rule that a fix belongs at the level where the parser doesn't
//! already stand in the way. Probing `pulldown-cmark` showed a
//! `Link`/`Image` destination containing a raw space is not
//! CommonMark-valid syntax at all when it isn't wrapped in `<...>` — such a
//! destination never becomes a `Link`/`Image` event to begin with (it
//! parses as literal bracket text), so there is nothing here to normalize;
//! that case is an upstream defect in each `un*` renderer's markdown
//! emission, not something a downstream pass can safely recover.
//! Backslashes and duplicate slashes,
//! by contrast, parse into a perfectly valid `Link`/`Image` event with the
//! raw destination preserved verbatim (verified empirically) — that valid-
//! but-messy shape is exactly this module's mandate, and is safe to rewrite
//! here with zero risk of ever creating or losing a link.
//!
//! **Never touches a destination containing `://` or starting with `//`.**
//! Extracted documents carry real hyperlinks (`https://...`) and
//! protocol-relative references (`//example.com/...`) whose own `//` is
//! meaningful, not accidental duplication — collapsing it would corrupt a
//! working URL. Everything else (a bare relative or absolute path) gets
//! backslashes converted to forward slashes and duplicate interior slashes
//! collapsed to one.

use pulldown_cmark::{CowStr, Event, Tag};

/// Rewrites every `Link`/`Image` event's `dest_url` in `events` in place,
/// returning the same events with only that field possibly changed.
pub(crate) fn normalize_link_destinations(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    events.into_iter().map(rewrite_event).collect()
}

fn rewrite_event(event: Event<'_>) -> Event<'_> {
    match event {
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: normalize(&dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: normalize(&dest_url),
            title,
            id,
        }),
        other => other,
    }
}

fn normalize(url: &str) -> CowStr<'static> {
    if url.contains("://") || url.starts_with("//") {
        return CowStr::from(url.to_string());
    }
    let forward_slashes = url.replace('\\', "/");
    let collapsed = collapse_duplicate_slashes(&forward_slashes);
    CowStr::from(collapsed)
}

fn collapse_duplicate_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_was_slash = false;
    for c in s.chars() {
        if c == '/' {
            if prev_was_slash {
                continue;
            }
            prev_was_slash = true;
        } else {
            prev_was_slash = false;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refine::roundtrip::{parse, serialize};

    fn refine_events(input: &str) -> String {
        let events = normalize_link_destinations(parse(input));
        serialize(events).unwrap()
    }

    #[test]
    fn converts_backslashes_to_forward_slashes() {
        let output = refine_events(r"[text](my\folder\file.png)");
        assert_eq!(output, "[text](my/folder/file.png)");
    }

    #[test]
    fn collapses_duplicate_slashes() {
        let output = refine_events("[text](folder//sub///file.png)");
        assert_eq!(output, "[text](folder/sub/file.png)");
    }

    #[test]
    fn normalizes_image_destinations_too() {
        let output = refine_events(r"![alt](my\folder\img.png)");
        assert_eq!(output, "![alt](my/folder/img.png)");
    }

    #[test]
    fn leaves_a_real_url_untouched() {
        let input = "[text](https://example.com//path)";
        assert_eq!(refine_events(input), input);
    }

    #[test]
    fn leaves_a_protocol_relative_reference_untouched() {
        let input = "[text](//example.com/path)";
        assert_eq!(refine_events(input), input);
    }

    #[test]
    fn leaves_an_already_clean_relative_path_untouched() {
        let input = "[text](folder/file.png)";
        assert_eq!(refine_events(input), input);
    }

    #[test]
    fn leaves_non_link_text_untouched() {
        let output = refine_events("just some text");
        assert_eq!(output, "just some text");
    }
}
