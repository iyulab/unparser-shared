//! AI-assisted markdown refine — a generative rewrite pass, distinct from the
//! crate's rule-based [`crate::refine`] (feature `refine`).
//!
//! Unlike that module, this one does **not** guarantee losslessness or idempotence —
//! a generative rewrite cannot honor either invariant by construction. It is a
//! separate opt-in stage meant to run *after* rule-based refine in a caller's
//! pipeline (deterministic corrections first, so there is less for the model to
//! second-guess), never a replacement for it.

use super::config::AiConfig;
use super::error::Error;
use super::http::{self, ChatMessage};

/// Options for [`refine_markdown`].
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct RefineAiOptions {
    /// Extra guidance appended to the system prompt (e.g. domain-specific
    /// terminology to preserve). Empty by default.
    pub instructions: Option<String>,
}

impl RefineAiOptions {
    /// Sets [`Self::instructions`] — `#[non_exhaustive]` blocks struct-literal
    /// construction from outside this crate, so this is how a caller sets it.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }
}

const SYSTEM_PROMPT: &str = "You clean up Markdown produced by an automated document \
converter. Fix awkward phrasing, spacing, and structure that a human editor would \
fix, without changing the document's meaning, adding information, or removing any \
content. Respond with the revised Markdown only — no commentary, no code fence \
around the whole response.";

/// Rewrites `markdown` with a generative pass, per `options`.
///
/// Returns the model's response verbatim (trimmed of leading/trailing whitespace).
/// Callers that need a lossless/idempotent guarantee should run
/// [`crate::refine::refine`] (feature `refine`) instead, or in addition, before this.
pub fn refine_markdown(
    cfg: &AiConfig,
    markdown: &str,
    options: &RefineAiOptions,
) -> Result<String, Error> {
    let system = match &options.instructions {
        Some(extra) => format!("{SYSTEM_PROMPT}\n\n{extra}"),
        None => SYSTEM_PROMPT.to_string(),
    };
    let messages = vec![
        ChatMessage::text("system", system),
        ChatMessage::text("user", markdown),
    ];
    let content = http::call(cfg, messages, false)?;
    Ok(content.trim().to_string())
}
