# Changelog

## Unreleased

### Added

- `csv::to_csv(rows)` and `csv::to_delimited(rows, delimiter)` — a table as CSV (RFC 4180) or
  tab-separated text. Rows are given as `csv::Cell { text, row_span, col_span }`, a merged cell
  recorded once on the cell that owns it; the writer lays them on the grid with the merged
  cell's value at its top-left position and the positions it covers empty, so every record has
  the same number of fields and no value is counted twice. A field holding the delimiter, a
  double quote or a line break is quoted with quotes doubled; records end with CRLF.

## 0.6.0

### Added

- `markdown::code_span(text, in_table_cell)` — a code span that reads back as exactly `text`.
  Nothing inside a code span is escaped (a backslash there is printed), so the span is fenced
  instead: one backtick longer than the longest run of backticks in the text, padded with a
  space when the text starts or ends with a backtick or both starts and ends with a space. A
  line ending becomes a space, and inside a table cell `|` is escaped. Empty text gives an empty
  string.

## 0.5.0

### Added

- `markdown::link(label, destination, title, in_table_cell)` — an inline link, with the label
  (inline Markdown the writer has already rendered) kept inside the link whatever the writer's
  own escaping does: an unescaped `[` or `]` is escaped (`see [3]` used to end the link text at
  `]`), a line ending becomes a space (a blank line ended the paragraph mid-link), a trailing
  backslash is doubled so it cannot escape the closing `]`, and inside a table cell an
  unescaped `|` is escaped, code spans included. Code spans and the escapes the label already
  carries are left as they are. A non-empty title is written in double quotes with `"`, a
  backslash that would escape the next character, and an `&` that starts a character reference
  escaped; a line ending becomes a space, and `|` is escaped inside a table cell.

## 0.4.0

### Added

- `markdown::image(alt, destination, in_table_cell)` — a picture as `![alt](destination)`, with
  the description flattened to one line (every whitespace run, blank lines included, becomes
  one space) and the characters that would end or change the link text escaped: `\`, `[`, `]`,
  `` ` ``, `<`, and an `&` that starts a character reference; `|` too inside a table cell.
- `markdown::link_destination(url, in_table_cell)` — a link or image destination that CommonMark
  reads back as exactly `url`: the bare form when it can carry it, otherwise `<...>` (spaces,
  `<`/`>`, control characters, or parentheses that do not balance — `notes).txt` used to end the
  destination at the `)`). A backslash that would escape the next character, and an `&` that
  starts a character reference, are escaped; a line ending is percent-encoded; `|` is escaped
  inside a table cell. An empty URL is `<>`.

## 0.3.0

### Added

- `markdown::emphasis_span` — the part of a styled run that emphasis delimiters (`*`, `**`,
  `~~`) can wrap, given the characters the run lands between. Whitespace stays outside, and so
  does punctuation that touches a letter or digit: an italic `", s"` after `32` is written
  `32, *s*`, not `32*, s*`, which CommonMark does not read as emphasis. A run with nothing
  wrappable gets `None`. Backslash escapes are never split. `std`-only, in the crate root.

## 0.2.0

### Added

- `ai` (opt-in feature, native only) — VLM-based image understanding and
  AI-assisted markdown refine:
  - `understand_image` — one vision call that judges an image as a text/table
    document scan (structured extraction, with `rowspan`/`colspan` preserved
    for merged cells) or a general image (a context-aware description using
    surrounding text).
  - `refine_markdown` — a generative rewrite pass, separate from the `refine`
    feature's rule-based one (a generative call cannot honor that module's
    lossless/idempotent invariants).
  - Both inert until a caller supplies an `AiConfig`; a transport failure or a
    truncated response (`finish_reason == "length"`, never treated as
    success) retries with exponential backoff, up to `AiConfig::max_retries`.
  - Not available on `wasm32-unknown-unknown` — its HTTP client does not
    target wasm.

## 0.1.0

### Added

- `ffi`, `kind`, `scaffold` — vendored from [`uncore`](https://crates.io/crates/uncore)
  0.2.0 verbatim (no behavior change; only self-referential doc-comment paths were
  updated to the new crate name). `uncore` itself will be archived once `unpdf`/`undoc`/
  `unhwp` adopt it.
- `refine` (opt-in feature) — vendored from [`unrefine`](https://crates.io/crates/unrefine)
  0.1.0 verbatim (no behavior change; self-referential doc-comment paths and internal
  cross-references were updated for the new crate/module identity). The markdown engine
  moved to `pulldown-cmark` 0.13 / `pulldown-cmark-to-cmark` 22 (from 0.12/18), with the
  golden snapshots unchanged. `unrefine` itself will be archived once `unpdf`/`undoc`/
  `unhwp` switch their dependency over.
