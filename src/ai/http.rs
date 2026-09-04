//! The `/chat/completions` call shared by [`super::image::understand_image`] and
//! [`super::refine::refine_markdown`]: request/response schema, retry, backoff.
//!
//! # Retry policy
//!
//! Only two outcomes are retried, both named by the design this module implements:
//! a transport failure (DNS/connect/TLS/read failure — the request never got a
//! complete response) and a truncated response (`finish_reason == "length"`, which
//! this module never treats as success — a table or JSON body cut off mid-stream is
//! silently wrong, not partially right). A non-success HTTP status and a malformed
//! response body are returned immediately: retrying the identical request against a
//! definitive server response is not a transient-failure recovery, it is a second
//! guess, and this module leaves that judgment call to the caller.

use std::thread::sleep;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::config::AiConfig;
use super::error::Error;

#[derive(Serialize)]
pub(crate) struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Serialize)]
pub(crate) struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

#[derive(Serialize)]
pub(crate) struct ChatMessage {
    pub role: &'static str,
    pub content: Vec<ContentPart>,
}

impl ChatMessage {
    pub(crate) fn text(role: &'static str, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlPart },
}

#[derive(Serialize)]
pub(crate) struct ImageUrlPart {
    pub url: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: String,
}

/// Encodes `bytes` as a `data:` URI for a vision message's `image_url` content part.
pub(crate) fn data_uri(bytes: &[u8], mime_type: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime_type};base64,{encoded}")
}

/// Calls `{base_url}/chat/completions` with `messages`, retrying per the policy
/// documented on this module, and returns the assistant message's `content` string.
///
/// `json_mode` requests `response_format: {"type": "json_object"}` — set by
/// [`super::image::understand_image`], left unset by [`super::refine::refine_markdown`]
/// (which wants free-form markdown back, not a JSON envelope around it).
pub(crate) fn call(
    cfg: &AiConfig,
    messages: Vec<ChatMessage>,
    json_mode: bool,
) -> Result<String, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(cfg.timeout))
        .http_status_as_error(false)
        .build()
        .into();

    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let request = ChatRequest {
        model: &cfg.model,
        messages,
        response_format: json_mode.then_some(ResponseFormat {
            kind: "json_object",
        }),
    };

    let max_attempts = cfg.max_retries.max(1);
    for attempt in 1..=max_attempts {
        let last_attempt = attempt == max_attempts;
        match send_once(&agent, &url, &cfg.api_key, &request) {
            Ok(SendOutcome::Content(text)) => return Ok(text),
            Ok(SendOutcome::Truncated) => {
                if last_attempt {
                    return Err(Error::Truncated);
                }
                sleep(backoff_delay(attempt));
            }
            Err(Error::Transport(reason)) => {
                if last_attempt {
                    return Err(Error::RetriesExhausted {
                        attempts: max_attempts,
                        last_error: Box::new(Error::Transport(reason)),
                    });
                }
                sleep(backoff_delay(attempt));
            }
            Err(non_retryable) => return Err(non_retryable),
        }
    }
    unreachable!("the loop above always returns by the last attempt")
}

enum SendOutcome {
    Content(String),
    Truncated,
}

fn send_once(
    agent: &ureq::Agent,
    url: &str,
    api_key: &str,
    request: &ChatRequest<'_>,
) -> Result<SendOutcome, Error> {
    let mut response = agent
        .post(url)
        .header("Authorization", &format!("Bearer {api_key}"))
        .send_json(request)
        .map_err(Error::transport)?;

    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(Error::transport)?;

    if !status.is_success() {
        return Err(Error::Status {
            code: status.as_u16(),
            body,
        });
    }

    let parsed: ChatCompletionResponse = serde_json::from_str(&body)
        .map_err(|e| Error::MalformedResponse(format!("chat completion envelope: {e}")))?;
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| Error::MalformedResponse("response had no choices".into()))?;

    if choice.finish_reason.as_deref() == Some("length") {
        return Ok(SendOutcome::Truncated);
    }
    Ok(SendOutcome::Content(choice.message.content))
}

/// Exponential backoff starting at 200ms, doubling per attempt, capped at 5s — kept
/// short relative to the org prototype's observed multi-second-to-minute call
/// latency, since the wait is between attempts of an already-slow call, not a
/// tight retry loop.
fn backoff_delay(attempt: u32) -> Duration {
    let millis = 200u64.saturating_mul(1u64 << (attempt - 1).min(16));
    Duration::from_millis(millis).min(Duration::from_secs(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_delay(1), Duration::from_millis(200));
        assert_eq!(backoff_delay(2), Duration::from_millis(400));
        assert_eq!(backoff_delay(3), Duration::from_millis(800));
        assert_eq!(backoff_delay(10), Duration::from_secs(5));
    }

    #[test]
    fn data_uri_shapes_a_base64_data_url() {
        let uri = data_uri(b"hi", "image/png");
        assert!(uri.starts_with("data:image/png;base64,"));
        assert_eq!(uri, "data:image/png;base64,aGk=");
    }
}
