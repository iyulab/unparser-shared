//! Configuration shared by both `ai` capabilities.

use std::time::Duration;

/// Configuration for [`super::understand_image`] and [`super::refine_markdown`].
///
/// Constructed with [`AiConfig::new`], then adjusted by setting the remaining public
/// fields directly (the struct is `#[non_exhaustive]` only to keep adding a field
/// non-breaking — it does not restrict field access on an existing value).
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct AiConfig {
    /// The OpenAI-compatible endpoint's base URL (no trailing `/chat/completions`).
    pub base_url: String,
    /// Bearer token sent as `Authorization: Bearer {api_key}`.
    pub api_key: String,
    /// The model name to request.
    pub model: String,
    /// Which images a caller sends through [`super::understand_image`].
    ///
    /// Read by the caller deciding what to send (e.g. `unpdf`'s wiring) —
    /// `unparser_shared::ai` itself never inspects this field. It exists on this
    /// struct so a single `AiConfig` can carry the setting end-to-end from a CLI flag
    /// or binding option down to the call site that needs it.
    pub image_scope: ImageScope,
    /// Per-attempt timeout. Deliberately generous by default — a page or image dense
    /// with tables/formulas has been observed taking tens of seconds to a few
    /// minutes.
    pub timeout: Duration,
    /// Maximum number of attempts (the first try plus retries) before giving up.
    /// Applies to both transport failures and a truncated (`finish_reason == "length"`)
    /// response.
    pub max_retries: u32,
}

impl AiConfig {
    /// Constructs a config with the required fields and the documented defaults for
    /// everything else (`image_scope: All`, `timeout: 120s`, `max_retries: 3`).
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            image_scope: ImageScope::All,
            timeout: Duration::from_secs(120),
            max_retries: 3,
        }
    }
}

/// Which images [`super::understand_image`] is meant to be called for.
///
/// Purely a signal the caller reads before deciding to call — see
/// [`AiConfig::image_scope`]'s docs for why this module never inspects it itself.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageScope {
    /// Every parsed image resource. Quality-first — the default once a caller
    /// supplies an [`AiConfig`] at all.
    #[default]
    All,
    /// Only pages a low-confidence OCR gate already flagged as a full-page scan with
    /// no extractable text layer. Narrows cost/latency for a caller that wants VLM
    /// calls only where the existing text extraction produced nothing.
    LowConfidencePagesOnly,
}
