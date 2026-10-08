//! What `markdown::image` and `markdown::link_destination` write, read back by a CommonMark
//! parser: exactly one image, whose alt text and destination are the ones given.

use proptest::prelude::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unparser_shared::markdown::{image, link_destination};

/// The images in `markdown`, each as (alt text, destination).
fn images(markdown: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut current: Option<(String, String)> = None;
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                current = Some((String::new(), dest_url.to_string()))
            }
            Event::End(TagEnd::Image) => found.extend(current.take()),
            Event::Text(text) | Event::Code(text) => {
                if let Some((alt, _)) = current.as_mut() {
                    alt.push_str(&text);
                }
            }
            _ => {}
        }
    }
    found
}

/// The destination a parser should read back: the URL, its line endings percent-encoded.
fn expected_destination(url: &str) -> String {
    url.replace('\n', "%0A").replace('\r', "%0D")
}

fn flattened(alt: &str) -> String {
    alt.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_multi_paragraph_description_stays_one_image() {
    let md = image(
        "A person standing in front of a building\n\nAI-generated content may be incorrect.",
        "image1.jpeg",
        false,
    );
    assert_eq!(
        images(&md),
        [(
            "A person standing in front of a building AI-generated content may be incorrect."
                .to_string(),
            "image1.jpeg".to_string()
        )]
    );
}

#[test]
fn destinations_that_the_bare_form_cannot_carry() {
    for url in [
        "notes).txt",
        "a(b",
        "my folder/file.png",
        "a<b>c d",
        "",
        "C:\\dir\\file.png",
        "trailing\\",
        "a\\(b",
        "line\nbreak",
        "?a=1&amp;b=2",
        "tab\there",
    ] {
        let md = image("x", url, false);
        assert_eq!(
            images(&md),
            [("x".to_string(), expected_destination(url))],
            "{url:?} written as {md:?}"
        );
    }
}

#[test]
fn bare_destinations_stay_bare() {
    assert_eq!(
        link_destination("https://example.com/a(b)c", false),
        "https://example.com/a(b)c"
    );
    assert_eq!(
        link_destination("C:\\dir\\file.png", false),
        "C:\\dir\\file.png"
    );
    assert_eq!(link_destination("?a=1&b=2", false), "?a=1&b=2");
}

#[test]
fn an_image_in_a_table_cell_keeps_the_cell() {
    let md = format!(
        "| a | b |\n|---|---|\n| {} | z |\n",
        image("p | q", "i.png", true)
    );
    let cells = Parser::new_ext(&md, Options::ENABLE_TABLES)
        .filter(|e| matches!(e, Event::Start(Tag::TableCell)))
        .count();
    assert_eq!(cells, 4, "{md}");
    assert_eq!(images(&md), [("p | q".to_string(), "i.png".to_string())]);
}

/// Alt text the escaping is meant to carry through unchanged (after whitespace flattening):
/// brackets, backslashes, backticks, angle brackets, character references, pipes, letters
/// from any script. `*`, `_` and `~` are excluded — a matched pair is emphasis by design.
fn alt_text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a",
            "Z",
            "7",
            " ",
            "\n",
            "\n\n",
            "\t",
            "[",
            "]",
            "\\",
            "`",
            "``",
            "<",
            ">",
            "<b>",
            "&",
            "&amp;",
            "&#35;",
            "&#x41;",
            "|",
            "(",
            ")",
            "!",
            "#",
            "-",
            "가",
            "한",
            "é",
            ":",
            "/",
            ".",
            "http://x.y",
        ]),
        0..12,
    )
    .prop_map(|parts| parts.concat())
}

fn url_text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a", "Z", "7", " ", "(", ")", "<", ">", "\\", "\\(", "&", "&amp;", "&#35;", "%20", "/",
            ".", ":", "?", "=", "#", "가", "\n", "\t", "\"", "'", "|", "a|b",
        ]),
        0..12,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn any_alt_and_destination_read_back_as_one_image(alt in alt_text(), url in url_text()) {
        let md = image(&alt, &url, false);
        prop_assert_eq!(
            images(&md),
            vec![(flattened(&alt), expected_destination(&url))],
            "written as {:?}", md
        );
    }

    #[test]
    fn inside_a_table_cell_too(alt in alt_text(), url in url_text()) {
        let md = format!("| h |\n|---|\n| {} |\n", image(&alt, &url, true));
        prop_assert_eq!(
            images(&md),
            vec![(flattened(&alt), expected_destination(&url))],
            "written as {:?}", md
        );
    }
}
