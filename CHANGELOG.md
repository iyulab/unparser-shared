# Changelog

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
