//! Integration tests for `ai::understand_image`, against a local mock server.

mod common;

use common::MockServer;
use unparser_shared::ai::{AiConfig, ContentBlock, ImageContext, ImageUnderstanding};

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
fn structured_response_preserves_merged_cells() {
    let inner = serde_json::json!({
        "kind": "structured",
        "blocks": [
            {"type": "paragraph", "text": "Q3 Revenue by Region"},
            {
                "type": "table",
                "header_rows": 1,
                "rows": [
                    [
                        {"text": "Region", "rowspan": 1, "colspan": 1},
                        {"text": "Revenue", "rowspan": 1, "colspan": 2}
                    ],
                    [
                        {"text": "APAC", "rowspan": 1, "colspan": 1},
                        {"text": "$1.2M", "rowspan": 1, "colspan": 1},
                        {"text": "+8%", "rowspan": 1, "colspan": 1}
                    ]
                ]
            }
        ]
    })
    .to_string();
    let server = MockServer::serving(vec![(200, chat_response("stop", &inner))]);

    let result = unparser_shared::ai::understand_image(
        &config(server.url(), 3),
        b"fake-image-bytes",
        "image/png",
        ImageContext::default(),
    )
    .expect("should succeed");

    let ImageUnderstanding::Structured(blocks) = result else {
        panic!("expected Structured, got {result:?}");
    };
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0],
        ContentBlock::Paragraph("Q3 Revenue by Region".into())
    );
    let ContentBlock::Table(table) = &blocks[1] else {
        panic!("expected a Table block");
    };
    assert_eq!(table.header_rows, 1);
    assert_eq!(
        table.rows[0][1].colspan, 2,
        "merged header cell must round-trip"
    );
    assert_eq!(table.rows[1][0].text, "APAC");
}

#[test]
fn description_response_for_a_general_image() {
    let inner =
        serde_json::json!({"kind": "description", "text": "A bar chart of quarterly revenue."})
            .to_string();
    let server = MockServer::serving(vec![(200, chat_response("stop", &inner))]);

    let result = unparser_shared::ai::understand_image(
        &config(server.url(), 3),
        b"fake-image-bytes",
        "image/png",
        ImageContext {
            preceding_text: Some("Figure 3 shows revenue trends."),
            following_text: None,
        },
    )
    .expect("should succeed");

    assert_eq!(
        result,
        ImageUnderstanding::Description("A bar chart of quarterly revenue.".into())
    );
}

#[test]
fn truncated_response_retries_then_fails_as_truncated() {
    // Two attempts, both truncated — max_retries: 2 means no third attempt.
    let server = MockServer::serving(vec![
        (200, chat_response("length", "{}")),
        (200, chat_response("length", "{}")),
    ]);

    let err = unparser_shared::ai::understand_image(
        &config(server.url(), 2),
        b"fake-image-bytes",
        "image/png",
        ImageContext::default(),
    )
    .expect_err("should fail");

    assert!(matches!(err, unparser_shared::ai::Error::Truncated));
}

#[test]
fn truncated_then_success_recovers_on_retry() {
    let inner = serde_json::json!({"kind": "description", "text": "recovered"}).to_string();
    let server = MockServer::serving(vec![
        (200, chat_response("length", "{}")),
        (200, chat_response("stop", &inner)),
    ]);

    let result = unparser_shared::ai::understand_image(
        &config(server.url(), 3),
        b"fake-image-bytes",
        "image/png",
        ImageContext::default(),
    )
    .expect("second attempt should succeed");

    assert_eq!(result, ImageUnderstanding::Description("recovered".into()));
}

#[test]
fn malformed_json_content_is_reported_distinctly() {
    let server = MockServer::serving(vec![(200, chat_response("stop", "not json at all"))]);

    let err = unparser_shared::ai::understand_image(
        &config(server.url(), 1),
        b"fake-image-bytes",
        "image/png",
        ImageContext::default(),
    )
    .expect_err("should fail");

    assert!(matches!(
        err,
        unparser_shared::ai::Error::MalformedResponse(_)
    ));
}

#[test]
fn non_success_status_is_reported_with_body() {
    let server = MockServer::serving(vec![(401, "{\"error\":\"invalid api key\"}".to_string())]);

    let err = unparser_shared::ai::understand_image(
        &config(server.url(), 3),
        b"fake-image-bytes",
        "image/png",
        ImageContext::default(),
    )
    .expect_err("should fail");

    match err {
        unparser_shared::ai::Error::Status { code, body } => {
            assert_eq!(code, 401);
            assert!(body.contains("invalid api key"));
        }
        other => panic!("expected Error::Status, got {other:?}"),
    }
}
