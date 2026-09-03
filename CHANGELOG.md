# Changelog

## 0.1.0

### Added

- `ffi`, `kind`, `scaffold` — vendored from [`uncore`](https://crates.io/crates/uncore)
  0.2.0 verbatim (no behavior change; only self-referential doc-comment paths were
  updated to the new crate name). `uncore` itself will be archived once `unpdf`/`undoc`/
  `unhwp` adopt it.
- `refine` (opt-in feature) — vendored from [`unrefine`](https://crates.io/crates/unrefine)
  0.1.0 verbatim (no behavior change; self-referential doc-comment paths and internal
  cross-references were updated for the new crate/module identity). `unrefine` itself will
  be archived once `unpdf`/`undoc`/`unhwp` switch their dependency over.
