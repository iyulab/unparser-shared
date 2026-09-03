//! YAML frontmatter extraction, so `pulldown-cmark` — which reads a bare
//! leading `---` line as a setext-heading underline — never has to see it.

/// Splits a leading YAML frontmatter block off `input`.
///
/// Returns `(frontmatter, remainder)`: `frontmatter` is `Some` verbatim slice
/// (opening/closing delimiters included) when `input` opens with a `---` line
/// and a matching closing `---` or `...` line follows; `remainder` is
/// everything after that block, with leading blank lines trimmed.
/// `frontmatter` is `None` — and `remainder` is all of `input` — when no such
/// block is found.
pub(crate) fn extract(input: &str) -> (Option<&str>, &str) {
    let leading_ws = input.len() - input.trim_start().len();
    let trimmed = &input[leading_ws..];

    if !trimmed.starts_with("---") {
        return (None, input);
    }

    let mut lines = trimmed.split_inclusive('\n');
    let opening = lines.next().unwrap_or_default();
    if opening.trim_end() != "---" {
        return (None, input);
    }

    let mut body_len = opening.len();
    let mut closing_len = 0;
    for line in lines {
        if matches!(line.trim_end_matches(['\n', '\r']), "---" | "...") {
            closing_len = line.len();
            break;
        }
        body_len += line.len();
    }

    if closing_len == 0 {
        return (None, input);
    }

    let end = leading_ws + body_len + closing_len;
    let remainder = input[end..].trim_start_matches(['\n', '\r']);
    (Some(&input[leading_ws..end]), remainder)
}

/// A frontmatter entry's value: a same-line scalar (`key: value`), or a
/// block sequence (`key:` followed by indented `- item` lines). These are
/// the only two shapes the three `un*` renderers emit — a same-line quoted
/// or bare scalar for every field except a keyword/tag list, which one of
/// them (only) emits as a one-level `- "item"` block. Nothing here
/// interprets a value's content (quotes, escapes, numbers): each is kept as
/// the exact text found after the delimiter, so normalization can never
/// corrupt a value it doesn't understand.
enum Value<'a> {
    Scalar(&'a str),
    List(Vec<&'a str>),
}

struct Entry<'a> {
    key: &'a str,
    value: Value<'a>,
}

/// Normalizes a frontmatter block extracted by [`extract`]: canonicalizes
/// `key`/`:`/value spacing, and drops an entry that exactly repeats an
/// earlier one (same key **and** same value) or a list item that exactly
/// repeats an earlier item in the same list. Key order is left as authored
/// — `unpdf`'s frontmatter deliberately puts `title` first, and nothing
/// asked for alphabetical order.
///
/// A same-key entry with a *different* value is never dropped: only an
/// exact duplicate carries no unique content, so only an exact duplicate is
/// safe to remove under the lossless invariant.
///
/// Falls back to returning `block` unchanged for anything not shaped like
/// one of the two [`Value`] forms above (a blank line inside the block, a
/// nested value, a nested `- item` list, ...) — normalizing a shape this
/// module doesn't model risks corrupting it, and no renderer in this family
/// produces such a shape today.
pub(crate) fn normalize(block: &str) -> String {
    let mut lines = block.split_inclusive('\n');
    let Some(opening) = lines.next() else {
        return block.to_string();
    };
    if opening.trim_end() != "---" {
        return block.to_string();
    }

    let mut body_lines: Vec<&str> = Vec::new();
    let mut closing: Option<&str> = None;
    for line in lines {
        if matches!(line.trim_end_matches(['\n', '\r']), "---" | "...") {
            closing = Some(line);
            break;
        }
        body_lines.push(line);
    }
    let Some(closing) = closing else {
        return block.to_string();
    };

    let Some(entries) = parse_entries(&body_lines) else {
        return block.to_string();
    };

    let mut out = String::from(opening);
    for entry in dedup_entries(entries) {
        match entry.value {
            Value::Scalar(v) => {
                out.push_str(entry.key);
                out.push_str(": ");
                out.push_str(v);
                out.push('\n');
            }
            Value::List(items) => {
                out.push_str(entry.key);
                out.push_str(":\n");
                for item in items {
                    out.push_str("  - ");
                    out.push_str(item);
                    out.push('\n');
                }
            }
        }
    }
    out.push_str(closing);
    out
}

fn parse_entries<'a>(body_lines: &[&'a str]) -> Option<Vec<Entry<'a>>> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i < body_lines.len() {
        let content = body_lines[i].trim_end_matches(['\n', '\r']);
        if content.trim().is_empty() || content.starts_with([' ', '\t']) {
            return None;
        }
        let colon = content.find(':')?;
        let key = content[..colon].trim_end();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return None;
        }
        let rest = content[colon + 1..].trim_start();
        i += 1;

        if rest.is_empty() {
            let mut items = Vec::new();
            while i < body_lines.len() {
                let item_content = body_lines[i].trim_end_matches(['\n', '\r']);
                let item_trimmed = item_content.trim_start();
                if item_content.starts_with("  ") && item_trimmed.starts_with("- ") {
                    items.push(&item_trimmed[2..]);
                    i += 1;
                } else {
                    break;
                }
            }
            if items.is_empty() {
                // A bare `key:` with nothing recognizable under it — not one
                // of the two shapes this module models.
                return None;
            }
            entries.push(Entry {
                key,
                value: Value::List(items),
            });
        } else {
            entries.push(Entry {
                key,
                value: Value::Scalar(rest),
            });
        }
    }
    Some(entries)
}

/// Drops an entry that exactly repeats an earlier `(key, value)` pair, and
/// a list item that exactly repeats an earlier item within the same entry.
/// Order-preserving (first occurrence wins) — order-preservation is what
/// keeps a second `normalize` pass a no-op, since there are no duplicates
/// left for it to find.
fn dedup_entries(entries: Vec<Entry<'_>>) -> Vec<Entry<'_>> {
    let mut seen: Vec<(&str, String)> = Vec::new();
    let mut out = Vec::new();
    for mut entry in entries {
        if let Value::List(items) = &mut entry.value {
            let mut item_seen = std::collections::HashSet::new();
            items.retain(|item| item_seen.insert(*item));
        }
        let canonical = match &entry.value {
            Value::Scalar(v) => format!("s:{v}"),
            Value::List(items) => format!("l:{}", items.join("\u{0}")),
        };
        if seen.iter().any(|(k, c)| *k == entry.key && *c == canonical) {
            continue;
        }
        seen.push((entry.key, canonical));
        out.push(entry);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_frontmatter_and_remainder() {
        let input = "---\ntitle: x\n---\n\n# Heading\n";
        let (fm, rest) = extract(input);
        assert_eq!(fm, Some("---\ntitle: x\n---\n"));
        assert_eq!(rest, "# Heading\n");
    }

    #[test]
    fn no_frontmatter_returns_input_unchanged() {
        let input = "# Heading\nbody\n";
        assert_eq!(extract(input), (None, input));
    }

    #[test]
    fn unterminated_frontmatter_is_not_extracted() {
        let input = "---\ntitle: x\nbody\n";
        assert_eq!(extract(input), (None, input));
    }

    #[test]
    fn accepts_dots_closing_delimiter() {
        let input = "---\ntitle: x\n...\nbody\n";
        let (fm, rest) = extract(input);
        assert_eq!(fm, Some("---\ntitle: x\n...\n"));
        assert_eq!(rest, "body\n");
    }

    #[test]
    fn tolerates_leading_blank_lines() {
        let input = "\n\n---\ntitle: x\n---\nbody\n";
        let (fm, rest) = extract(input);
        assert_eq!(fm, Some("---\ntitle: x\n---\n"));
        assert_eq!(rest, "body\n");
    }

    #[test]
    fn dashes_inside_body_do_not_close_early() {
        let input = "---\ntitle: \"a --- b\"\n---\nbody\n";
        let (fm, rest) = extract(input);
        assert_eq!(fm, Some("---\ntitle: \"a --- b\"\n---\n"));
        assert_eq!(rest, "body\n");
    }

    #[test]
    fn normalize_leaves_an_already_canonical_block_unchanged() {
        let block = "---\ntitle: \"x\"\nauthor: \"y\"\n---\n";
        assert_eq!(normalize(block), block);
    }

    #[test]
    fn normalize_canonicalizes_colon_spacing() {
        assert_eq!(
            normalize("---\ntitle  :\"x\"\n---\n"),
            "---\ntitle: \"x\"\n---\n"
        );
    }

    #[test]
    fn normalize_drops_an_exact_duplicate_entry() {
        assert_eq!(
            normalize("---\ntitle: \"x\"\ntitle: \"x\"\n---\n"),
            "---\ntitle: \"x\"\n---\n"
        );
    }

    #[test]
    fn normalize_keeps_a_same_key_entry_with_a_different_value() {
        let block = "---\ntitle: \"x\"\ntitle: \"y\"\n---\n";
        assert_eq!(normalize(block), block);
    }

    #[test]
    fn normalize_preserves_authored_key_order() {
        // unpdf deliberately emits `title` before `author` for RAG-friendly
        // output -- normalize must not alphabetize (`author` < `title`).
        let block = "---\ntitle: \"x\"\nauthor: \"y\"\n---\n";
        assert_eq!(normalize(block), block);
    }

    #[test]
    fn normalize_round_trips_a_block_sequence() {
        // unhwp's `tags:` shape -- the only nested form any renderer emits.
        let block = "---\ntags:\n  - \"a\"\n  - \"b\"\n---\n";
        assert_eq!(normalize(block), block);
    }

    #[test]
    fn normalize_drops_an_exact_duplicate_list_item() {
        assert_eq!(
            normalize("---\ntags:\n  - \"a\"\n  - \"a\"\n  - \"b\"\n---\n"),
            "---\ntags:\n  - \"a\"\n  - \"b\"\n---\n"
        );
    }

    #[test]
    fn normalize_is_idempotent_on_a_mixed_block() {
        let block = "---\ntitle:\"x\"\ntitle: \"x\"\ntags:\n  - \"a\"\n  - \"a\"\n---\n";
        let once = normalize(block);
        let twice = normalize(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn normalize_falls_back_on_a_blank_line_inside_the_block() {
        let block = "---\ntitle: \"x\"\n\nauthor: \"y\"\n---\n";
        assert_eq!(normalize(block), block);
    }

    #[test]
    fn normalize_falls_back_on_an_unrecognized_bare_key() {
        // `key:` with nothing under it that looks like a `- item` line --
        // not one of the two shapes this module models.
        let block = "---\nkey:\nnext: \"x\"\n---\n";
        assert_eq!(normalize(block), block);
    }
}
