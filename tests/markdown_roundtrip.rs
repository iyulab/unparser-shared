//! What `markdown::image`, `markdown::link` and `markdown::link_destination` write, read back
//! by a CommonMark parser: exactly one image or link, whose text, destination and title are the
//! ones given.

use proptest::prelude::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unparser_shared::markdown::{image, link, link_destination};

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

/// Footnotes on, as the writers' output uses them (`[^1]`), so a label that could read as a
/// footnote reference is caught.
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES
}

/// The links in `markdown`, each as (text, destination, title). Code spans count as their
/// text; a soft break as a space.
fn links(markdown: &str) -> Vec<(String, String, String)> {
    let mut found = Vec::new();
    let mut current: Option<(String, String, String)> = None;
    for event in Parser::new_ext(markdown, options()) {
        match event {
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => current = Some((String::new(), dest_url.to_string(), title.to_string())),
            Event::End(TagEnd::Link) => found.extend(current.take()),
            Event::Text(text) | Event::Code(text) => {
                if let Some((t, _, _)) = current.as_mut() {
                    t.push_str(&text);
                }
            }
            Event::SoftBreak => {
                if let Some((t, _, _)) = current.as_mut() {
                    t.push(' ');
                }
            }
            _ => {}
        }
    }
    found
}

fn one_link(text: &str, dest: &str, title: &str) -> Vec<(String, String, String)> {
    vec![(text.to_string(), dest.to_string(), title.to_string())]
}

#[test]
fn a_label_with_brackets_stays_one_link() {
    for label in ["see [3]", "a ] b", "[", "x [y](z) w", "]]"] {
        let md = link(label, "https://example.com", None, false);
        assert_eq!(
            links(&md),
            one_link(label, "https://example.com", ""),
            "{md}"
        );
    }
}

#[test]
fn a_rendered_label_keeps_its_markup_and_escapes() {
    // Styling stays styling; an escape the writer made stays an escape.
    let md = link(r"**bold** and \*star\*", "u", None, false);
    assert_eq!(links(&md), one_link("bold and *star*", "u", ""), "{md}");
    let md = link(r"already \[escaped\]", "u", None, false);
    assert_eq!(md, r"[already \[escaped\]](u)");
    assert_eq!(links(&md), one_link("already [escaped]", "u", ""));
}

#[test]
fn a_code_span_in_the_label_is_left_as_it_is() {
    let md = link("call `f[0]` or ``a`]b``", "u", None, false);
    assert_eq!(md, "[call `f[0]` or ``a`]b``](u)");
    assert_eq!(links(&md), one_link("call f[0] or a`]b", "u", ""));
    // An unclosed backtick is literal, and the bracket after it is escaped.
    let md = link("a ` b ]", "u", None, false);
    assert_eq!(links(&md), one_link("a ` b ]", "u", ""), "{md}");
}

#[test]
fn a_blank_line_in_the_label_does_not_end_the_paragraph() {
    let md = link("first\n\nsecond", "u", None, false);
    assert_eq!(links(&md), one_link("first  second", "u", ""), "{md}");
}

#[test]
fn a_trailing_backslash_does_not_escape_the_closing_bracket() {
    let md = link(r"C:\dir\", "u", None, false);
    assert_eq!(links(&md), one_link(r"C:\dir\", "u", ""), "{md}");
}

/// The inline link wins over a footnote reference of the same label, so `^` needs no escape.
#[test]
fn a_caret_label_is_not_a_footnote_reference() {
    let md = format!("{}\n\n[^1]: the note\n", link("^1", "u", None, false));
    assert_eq!(links(&md), one_link("^1", "u", ""), "{md}");
}

#[test]
fn titles_read_back_as_given() {
    for title in [
        r#"say "hi""#,
        r"back\slash",
        r"ends with \",
        r#"\""#,
        "&amp; stays",
        "two\nlines",
        "a | b",
    ] {
        let md = link("x", "u", Some(title), false);
        let expected = title.replace('\n', " ");
        assert_eq!(
            links(&md),
            one_link("x", "u", &expected),
            "{title:?} as {md}"
        );
    }
    assert_eq!(link("x", "u", Some(""), false), "[x](u)");
}

#[test]
fn a_link_in_a_table_cell_keeps_the_cell() {
    let md = format!(
        "| a | b |\n|---|---|\n| {} | z |\n",
        link("p | q `r|s`", "a|b", Some("t|u"), true)
    );
    let cells = Parser::new_ext(&md, options())
        .filter(|e| matches!(e, Event::Start(Tag::TableCell)))
        .count();
    assert_eq!(cells, 4, "{md}");
    assert_eq!(links(&md), one_link("p | q r|s", "a|b", "t|u"), "{md}");
}

/// Plain label text — no characters that style (`*`, `_`, `~`) or are taken as markup by the
/// writer's own escaping (`\`, `` ` ``, `<`, `&`) — which must read back as itself.
fn plain_label() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a", "Z", "7", " ", "\n", "\n\n", "[", "]", "[x](y)", "|", "(", ")", "!", "#", "-",
            "^", "가", "é", ":", "/", ".", "\"",
        ]),
        0..12,
    )
    .prop_map(|parts| parts.concat())
}

fn title_text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a", " ", "\"", "\\", "\\\"", "&", "&amp;", "&#35;", "\n", "|", "(", ")", "'", "가",
        ]),
        0..10,
    )
    .prop_map(|parts| parts.concat())
}

/// What a parser reads back from a label: line endings as spaces, and — since a paragraph
/// trims them — no whitespace at either end.
fn read_back_label(label: &str) -> String {
    label.replace(['\n', '\r'], " ").trim().to_string()
}

fn normalized(links: Vec<(String, String, String)>) -> Vec<(String, String, String)> {
    links
        .into_iter()
        .map(|(t, d, ti)| (t.trim().to_string(), d, ti))
        .collect()
}

proptest! {
    #[test]
    fn any_label_destination_and_title_read_back_as_one_link(
        label in plain_label(),
        url in url_text(),
        title in title_text(),
    ) {
        let md = link(&label, &url, Some(&title), false);
        prop_assert_eq!(
            normalized(links(&md)),
            one_link(&read_back_label(&label), &expected_destination(&url), &title.replace('\n', " ")),
            "written as {:?}", md
        );
    }

    #[test]
    fn a_link_inside_a_table_cell_too(label in plain_label(), url in url_text(), title in title_text()) {
        let md = format!("| h |\n|---|\n| {} |\n", link(&label, &url, Some(&title), true));
        prop_assert_eq!(
            normalized(links(&md)),
            one_link(&read_back_label(&label), &expected_destination(&url), &title.replace('\n', " ")),
            "written as {:?}", md
        );
    }
}
