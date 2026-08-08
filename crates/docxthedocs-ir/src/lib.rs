use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Converted,
    ConvertedWithWarnings,
    UnsupportedSource,
    InvalidSource,
    InternalError,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryCounts {
    pub main: u32,
    pub footnotes: u32,
    pub headers: u32,
    pub comments: u32,
    pub endnotes: u32,
    pub textboxes: u32,
    pub header_textboxes: u32,
}

impl StoryCounts {
    pub fn total(&self) -> Option<u32> {
        [
            self.main,
            self.footnotes,
            self.headers,
            self.comments,
            self.endnotes,
            self.textboxes,
            self.header_textboxes,
        ]
        .into_iter()
        .try_fold(0_u32, u32::checked_add)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub engine: String,
    pub version: String,
    pub source_sha256: String,
    pub status: Status,
    pub features_seen: BTreeSet<String>,
    pub unsupported: BTreeSet<String>,
    pub warnings: Vec<String>,
    pub stories: StoryCounts,
    pub stream_inventory: Vec<StreamInfo>,
    pub pieces: usize,
    pub paragraphs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamInfo {
    pub path: String,
    pub kind: StreamKind,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Root,
    Storage,
    Stream,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub paragraphs: Vec<Paragraph>,
    pub section: SectionProperties,
    pub numbering: Vec<NumberingDefinition>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paragraph {
    pub properties: ParagraphProperties,
    pub children: Vec<Inline>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParagraphProperties {
    pub bidi: Option<bool>,
    pub alignment: Option<Alignment>,
    pub indent_start: Option<i16>,
    pub indent_end: Option<i16>,
    pub indent_first_line: Option<i16>,
    pub space_before: Option<u16>,
    pub space_after: Option<u16>,
    pub numbering: Option<NumberingRef>,
    pub borders: ParagraphBorders,
    pub shading: Option<Shading>,
    pub table: TableParagraphProperties,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableParagraphProperties {
    pub in_table: bool,
    pub depth: u8,
    pub cell_end: bool,
    pub row_end: bool,
    pub rtl: bool,
    pub reverse_cells: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParagraphBorders {
    pub top: Option<Border>,
    pub start: Option<Border>,
    pub bottom: Option<Border>,
    pub end: Option<Border>,
    pub between: Option<Border>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Border {
    pub style: String,
    pub size: u8,
    pub space: u8,
    pub color: Option<String>,
    pub shadow: bool,
    pub frame: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shading {
    pub pattern: String,
    pub foreground: Option<String>,
    pub background: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Both,
    Distribute,
    MediumKashida,
    HighKashida,
    LowKashida,
    ThaiDistribute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberingRef {
    pub instance: u32,
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberingDefinition {
    pub instance: u32,
    pub levels: Vec<NumberingLevel>,
    pub overrides: Vec<NumberingOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberingOverride {
    pub level: u8,
    pub start: Option<u32>,
    pub formatting: Option<NumberingLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberingLevel {
    pub level: u8,
    pub start: u32,
    pub format: String,
    pub text: String,
    pub alignment: Alignment,
    pub suffix: NumberingSuffix,
    pub paragraph: ParagraphProperties,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberingSuffix {
    Tab,
    Space,
    Nothing,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionProperties {
    pub page_width: Option<u16>,
    pub page_height: Option<u16>,
    pub margin_top: Option<i16>,
    pub margin_bottom: Option<i16>,
    pub margin_left: Option<u16>,
    pub margin_right: Option<u16>,
    pub landscape: Option<bool>,
    pub bidi: Option<bool>,
    pub rtl_gutter: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Symbol { font: String, character: u16 },
    Tab,
    Break(BreakKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    Line,
    Page,
    Column,
}
