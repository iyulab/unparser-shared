//! Integration tests for `ai::refine_markdown`, against a local mock server.

mod common;

use common::MockServer;
use unparser_shared::ai::{AiConfig, RefineAiOptions};

fn config(url: &str, max_retries: u32) -> AiConfig {
    let mut cfg = AiConfig::new(url, "test-key", "test-model");
    cfg.max_retries = max_retries;
    cfg
}

fn chat_response(finish_reason: &str, content: &str) -> String {
    serde_json::json!({
        "choices": [{
            "message": {"content": content},
            "finish_reason": finish_reason,
        }]
    })
    .to_string()
}

#[test]
fn returns_the_rewritten_markdown_trimmed() {
    let server = MockServer::serving(vec![(
        200,
        chat_response("stop", "\n# Cleaned Up\n\nBetter phrasing.\n"),
    )]);

    let result = unparser_shared::ai::refine_markdown(
        &config(server.url(), 1),
        "# messy\n\nawkward phrasing",
        &RefineAiOptions::default(),
    )
    .expect("should succeed");

    assert_eq!(result, "# Cleaned Up\n\nBetter phrasing.");
}

#[test]
fn instructions_do_not_change_the_call_shape_or_the_success_path() {
    let server = MockServer::serving(vec![(200, chat_response("stop", "revised"))]);

    let result = unparser_shared::ai::refine_markdown(
        &config(server.url(), 1),
        "original",
        &RefineAiOptions::default().with_instructions("Preserve all product names verbatim."),
    )
    .expect("should succeed");

    assert_eq!(result, "revised");
}

#[test]
fn a_truncated_first_attempt_recovers_on_retry() {
    let server = MockServer::serving(vec![
        (200, chat_response("length", "cut off mid")),
        (200, chat_response("stop", "complete")),
    ]);

    let result = unparser_shared::ai::refine_markdown(
        &config(server.url(), 3),
        "original",
        &RefineAiOptions::default(),
    )
    .expect("second attempt should succeed");

    assert_eq!(result, "complete");
}
