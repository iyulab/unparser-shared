# Changelog

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
