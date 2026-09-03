/// Options controlling which passes [`refine`](super::refine) runs.
///
/// Empty in `0.1.0` — no passes are implemented yet, so [`crate::refine::refine`]
/// currently performs only the CommonMark round trip (parse, then
/// re-serialize) that gives it the losslessness and idempotence guarantees
/// this crate exists to provide. Each landed pass adds a field here;
/// `#[non_exhaustive]` keeps that additive rather than breaking for anyone
/// constructing or matching on this struct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RefineOptions {}
