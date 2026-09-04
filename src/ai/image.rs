//! VLM-based image understanding — a single call that judges whether an image is a
//! text/table document scan (structured extraction) or a general image (a
//! context-aware description), and returns the matching branch.
//!
//! See the crate's `ai` module docs for why this is one call rather than a
//! classify-then-extract pair, and why the structured branch is JSON rather than a
//! free markdown string (GFM tables cannot express a merged cell).

use serde::Deserialize;

use super::config::AiConfig;
use super::error::Error;
use super::http::{self, ChatMessage, ContentPart, ImageUrlPart};

/// Text surrounding the image, for [`understand_image`]'s description branch — a
/// caption-less photo described with "a bar chart" is far less useful than one
/// described with the surrounding paragraph's subject folded in.
///
/// Both fields are optional and independent: a page boundary may have preceding
/// text but no following text (last image on a page) or vice versa.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageContext<'a> {
    /// Text immediately before the image (same page, or the previous page's last
    /// paragraph at a page boundary).
    pub preceding_text: Option<&'a str>,
    /// Text immediately after the image (same page, or the next page's first
    /// paragraph at a page boundary).
    pub following_text: Option<&'a str>,
}

/// What [`understand_image`] judged the image to be.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ImageUnderstanding {
    /// Judged a text/table document scan — structured extraction.
    Structured(Vec<ContentBlock>),
    /// Judged a general image (photo, chart, diagram) — a context-aware description.
    Description(String),
}

/// One block of content extracted from a [`ImageUnderstanding::Structured`] image.
///
/// Defined by this module, not borrowed from a consumer's document model — see the
/// crate's `ai` module docs on why the dependency direction runs consumer → this
/// module and not the other way around.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ContentBlock {
    /// A paragraph of text.
    Paragraph(String),
    /// A table, with merged-cell structure preserved.
    Table(TableBlock),
}

/// A table extracted from a [`ImageUnderstanding::Structured`] image.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TableBlock {
    /// Rows of cells, in document order.
    pub rows: Vec<Vec<TableCellBlock>>,
    /// How many leading rows are header rows.
    pub header_rows: u8,
}

/// One cell of a [`TableBlock`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TableCellBlock {
    /// The cell's text.
    pub text: String,
    /// How many rows this cell spans.
    pub rowspan: u8,
    /// How many columns this cell spans.
    pub colspan: u8,
}

/// Judges `image` as a document scan or a general image, and extracts or describes
/// it accordingly, in a single call.
///
/// `mime_type` is the image's own MIME type (e.g. `"image/png"`) — passed through
/// unmodified into the vision message's `data:` URI.
pub fn understand_image(
    cfg: &AiConfig,
    image: &[u8],
    mime_type: &str,
    context: ImageContext<'_>,
) -> Result<ImageUnderstanding, Error> {
    let messages = vec![
        ChatMessage::text(
            "system",
            "You are given one image. First decide: is it primarily a scan of a \
             text or table document, or a general image (a photo, chart, or \
             diagram)? Then respond with exactly one JSON object and nothing else.\n\n\
             If it is a document scan, respond with:\n\
             {\"kind\":\"structured\",\"blocks\":[\n\
               {\"type\":\"paragraph\",\"text\":\"...\"},\n\
               {\"type\":\"table\",\"header_rows\":1,\"rows\":[[{\"text\":\"...\",\
             \"rowspan\":1,\"colspan\":1}]]}\n\
             ]}\n\
             Preserve reading order, heading levels as plain text, and — this is the \
             point of asking for JSON instead of markdown — every merged cell's true \
             rowspan/colspan.\n\n\
             If it is a general image, respond with:\n\
             {\"kind\":\"description\",\"text\":\"...\"}\n\
             Write the description using the surrounding text as context where it \
             helps identify what the image shows.",
        ),
        vision_message(image, mime_type, context),
    ];

    let content = http::call(cfg, messages, true)?;
    let dto: UnderstandingDto = serde_json::from_str(&content)
        .map_err(|e| Error::MalformedResponse(format!("image understanding: {e}")))?;
    Ok(dto.into())
}

fn vision_message(image: &[u8], mime_type: &str, context: ImageContext<'_>) -> ChatMessage {
    let mut parts = Vec::new();
    if let Some(preceding) = context.preceding_text {
        parts.push(ContentPart::Text {
            text: format!("Text immediately before the image:\n{preceding}"),
        });
    }
    parts.push(ContentPart::ImageUrl {
        image_url: ImageUrlPart {
            url: http::data_uri(image, mime_type),
        },
    });
    if let Some(following) = context.following_text {
        parts.push(ContentPart::Text {
            text: format!("Text immediately after the image:\n{following}"),
        });
    }
    ChatMessage {
        role: "user",
        content: parts,
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum UnderstandingDto {
    Structured { blocks: Vec<ContentBlockDto> },
    Description { text: String },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlockDto {
    Paragraph {
        text: String,
    },
    Table {
        header_rows: u8,
        rows: Vec<Vec<TableCellDto>>,
    },
}

#[derive(Deserialize)]
struct TableCellDto {
    text: String,
    #[serde(default = "one")]
    rowspan: u8,
    #[serde(default = "one")]
    colspan: u8,
}

fn one() -> u8 {
    1
}

impl From<UnderstandingDto> for ImageUnderstanding {
    fn from(dto: UnderstandingDto) -> Self {
        match dto {
            UnderstandingDto::Description { text } => ImageUnderstanding::Description(text),
            UnderstandingDto::Structured { blocks } => {
                ImageUnderstanding::Structured(blocks.into_iter().map(ContentBlock::from).collect())
            }
        }
    }
}

impl From<ContentBlockDto> for ContentBlock {
    fn from(dto: ContentBlockDto) -> Self {
        match dto {
            ContentBlockDto::Paragraph { text } => ContentBlock::Paragraph(text),
            ContentBlockDto::Table { header_rows, rows } => ContentBlock::Table(TableBlock {
                header_rows,
                rows: rows
                    .into_iter()
                    .map(|row| row.into_iter().map(TableCellBlock::from).collect())
                    .collect(),
            }),
        }
    }
}

impl From<TableCellDto> for TableCellBlock {
    fn from(dto: TableCellDto) -> Self {
        TableCellBlock {
            text: dto.text,
            rowspan: dto.rowspan,
            colspan: dto.colspan,
        }
    }
}
