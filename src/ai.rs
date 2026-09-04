//! VLM-based image understanding and AI-assisted markdown refine — active only when
//! a caller supplies an [`AiConfig`]. Neither capability touches the network unless
//! called explicitly; there is no ambient behavior change for a caller that never
//! constructs a config.
//!
//! # The two capabilities
//!
//! | Capability | Input → output | Where a caller wires it |
//! |---|---|---|
//! | [`understand_image`] | image bytes → structured content or a description | The parsing stage, for an image whose surrounding text extraction produced nothing (or, at a caller's option, every embedded image) |
//! | [`refine_markdown`] | markdown → markdown | Render post-processing, after [`crate::refine::refine`] (feature `refine`) if that runs too |
//!
//! # Why this lives here and not in `crate::refine` or the crate root
//!
//! The crate root ([`crate::ffi`], [`crate::kind`], [`crate::scaffold`]) is linked
//! unconditionally by three published cdylibs and stays dependency-free by design —
//! an HTTP client and JSON serialization do not belong there. [`crate::refine`]
//! guarantees losslessness and idempotence for a deterministic pass; a generative
//! call cannot honor either invariant, so it is a separate feature rather than a
//! mode of that one.
//!
//! # Native only
//!
//! This feature is not available on `wasm32-unknown-unknown`: its HTTP client
//! (`ureq`) does not build for that target. A browser caller would need `fetch` JS
//! interop, which is not implemented here — see the crate's CI, which checks
//! `refine` on wasm32 but does not attempt `ai` there.
//!
//! # What this module does not do
//!
//! It does not decide *which* images to send, or *when* a page's extracted text
//! counts as "nothing" — those are domain judgments that belong to a caller
//! (`unpdf`'s wiring, say), not to this crate. [`AiConfig::image_scope`] exists so a
//! caller can carry that decision alongside the rest of the configuration; this
//! module never reads it.

mod config;
mod error;
mod http;
mod image;
mod refine;

pub use config::{AiConfig, ImageScope};
pub use error::Error;
pub use image::{
    understand_image, ContentBlock, ImageContext, ImageUnderstanding, TableBlock, TableCellBlock,
};
pub use refine::{refine_markdown, RefineAiOptions};
