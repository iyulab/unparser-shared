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

These three modules are always compiled and have **zero dependencies** — three published
cdylibs link this crate statically, so a dependency here is a dependency in all of them.

## On the roadmap

- `refine` (opt-in feature) — a lossless, idempotent markdown shape-refinement pass.
- `ai` (opt-in feature) — VLM-based image understanding (OCR or description, judged per
  image) and AI-assisted markdown refine, active only when the caller supplies an API key.

See [docs.rs](https://docs.rs/unparser-shared) for the current API.

## License

MIT
