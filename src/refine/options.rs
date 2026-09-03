/// Options controlling which passes [`refine`](super::refine) runs.
///
/// Empty in `0.1.0` — every pass currently runs unconditionally, with no way to
/// disable or configure any of them individually. `#[non_exhaustive]` keeps
/// adding a field here additive rather than breaking for anyone constructing or
/// matching on this struct, for whenever a pass needs a knob.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RefineOptions {}
