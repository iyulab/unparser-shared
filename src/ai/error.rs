//! The failure reasons this module's two entry points can return.
//!
//! Classification here stays purely diagnostic. This crate has no C ABI boundary of
//! its own — `unpdf`/`undoc`/`unhwp` each classify their own `ErrorKind` at the point
//! that knows the reason, so a caller wiring [`super::understand_image`] or
//! [`super::refine_markdown`] into its own error type maps from this enum, or (per the
//! fallback policy that governs the `unpdf` wiring) collapses every variant into a
//! single failure counter. Either way, the variants below stay specific — collapsing
//! them is the caller's decision to make, not this module's to make for it.

use std::fmt;

/// Why a call to [`super::understand_image`] or [`super::refine_markdown`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The request never reached the endpoint, or the connection dropped before a
    /// response arrived — DNS, connect, TLS, or read/write failure.
    #[error("transport failure reaching the AI endpoint: {0}")]
    Transport(String),

    /// The endpoint responded, but with a non-success HTTP status.
    #[error("AI endpoint responded with status {code}: {body}")]
    Status {
        /// The HTTP status code.
        code: u16,
        /// The response body, for diagnostics.
        body: String,
    },

    /// The response was truncated (`finish_reason == "length"`) even after every
    /// retry. Never treated as success — a truncated table or JSON body is silently
    /// wrong, not partially right.
    #[error("response was truncated (finish_reason=length) after all retries")]
    Truncated,

    /// The response body was not the shape this call expects — malformed JSON, or
    /// valid JSON that does not match the schema the prompt asked for.
    #[error("response did not match the expected schema: {0}")]
    MalformedResponse(String),

    /// Every attempt failed (transport failure or truncation) up to `max_retries`.
    #[error("all {attempts} attempt(s) failed; last error: {last_error}")]
    RetriesExhausted {
        /// How many attempts were made.
        attempts: u32,
        /// The error from the last attempt.
        last_error: Box<Error>,
    },
}

impl Error {
    pub(crate) fn transport(err: impl fmt::Display) -> Self {
        Self::Transport(err.to_string())
    }
}
