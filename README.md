# unparser-shared

Shared plumbing for the [`unpdf`](https://crates.io/crates/unpdf),
[`undoc`](https://crates.io/crates/undoc) and [`unhwp`](https://crates.io/crates/unhwp)
document-extraction family.

## What's here today

- `ffi` — thread-local last-error storage and a panic guard for a C ABI that returns
  sentinels rather than `Result`.
- `kind` — the error-kind numbering the family already shares, and the bands that keep
  future reasons from colliding.
- `scaffold` — macros that assemble a C entry point out of the two primitives above.
- `refine` (opt-in feature) — a lossless, idempotent markdown shape-refinement pass: table
  shape normalization, ordered-list renumbering, link/image path normalization, frontmatter
  normalization, and section anchors.
- `ai` (opt-in feature, native only) — VLM-based image understanding (structured
  extraction with merged-cell tables, or a context-aware description — judged per image
  in one call) and AI-assisted markdown refine. Both are inert until a caller supplies an
  `AiConfig`; neither touches the network otherwise. Not available on
  `wasm32-unknown-unknown` (its HTTP client does not target wasm).

`ffi`/`kind`/`scaffold` are always compiled and have **zero dependencies** — three published
cdylibs link this crate statically, so a dependency here is a dependency in all of them.
`refine` and `ai` are opt-in and bring their own dependencies only when enabled
(`pulldown-cmark`/`pulldown-cmark-to-cmark` for `refine`; `ureq`/`serde`/`serde_json`/`base64`/
`thiserror` for `ai`).

See [docs.rs](https://docs.rs/unparser-shared) for the current API.

## License

MIT
