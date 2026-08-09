use docxthedocs_ir::{
    Alignment, Border, BreakKind, Document, Inline, NumberingSuffix, Paragraph,
    ParagraphProperties, SectionProperties,
};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use std::fs::File;
use std::io::{self, Seek, Write};
use std::path::Path;
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

const CONTENT_TYPES_NUMBERING: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;

const DOCUMENT_RELS_NUMBERING: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr/></w:rPrDefault><w:pPrDefault><w:pPr/></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style></w:styles>"#;

#[derive(Debug, Error)]
pub enum OoxmlError {
    #[error("DOCX I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("DOCX ZIP creation failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("DOCX XML creation failed: {0}")]
    Xml(#[from] quick_xml::Error),
}

pub fn write_docx(path: &Path, document: &Document) -> Result<(), OoxmlError> {
    let file = File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let has_numbering = !document.numbering.is_empty();
    let content_types = if has_numbering {
        CONTENT_TYPES_NUMBERING
    } else {
        CONTENT_TYPES
    };
    write_part(&mut zip, "[Content_Types].xml", content_types.as_bytes())?;
    write_part(&mut zip, "_rels/.rels", ROOT_RELS.as_bytes())?;
    write_part(&mut zip, "word/document.xml", &document_xml(document)?)?;
    write_part(&mut zip, "word/styles.xml", STYLES.as_bytes())?;
    if has_numbering {
        write_part(&mut zip, "word/numbering.xml", &numbering_xml(document)?)?;
    }
    let document_rels = if has_numbering {
        DOCUMENT_RELS_NUMBERING
    } else {
        DOCUMENT_RELS
    };
    write_part(
        &mut zip,
        "word/_rels/document.xml.rels",
        document_rels.as_bytes(),
    )?;
    zip.finish()?.sync_all()?;
    Ok(())
}

fn write_part<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    name: &str,
    bytes: &[u8],
) -> Result<(), OoxmlError> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    zip.start_file(name, options)?;
    zip.write_all(bytes)?;
    Ok(())
}

fn document_xml(document: &Document) -> Result<Vec<u8>, OoxmlError> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        Some("yes"),
    )))?;
    let mut root = BytesStart::new("w:document");
    root.push_attribute((
        "xmlns:w",
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    ));
    writer.write_event(Event::Start(root))?;
    writer.write_event(Event::Start(BytesStart::new("w:body")))?;

    let mut index = 0_usize;
    while index < document.paragraphs.len() {
        let paragraph = &document.paragraphs[index];
        if paragraph.properties.table.in_table && paragraph.properties.table.depth == 1 {
            let start = index;
            while index < document.paragraphs.len()
                && document.paragraphs[index].properties.table.in_table
                && document.paragraphs[index].properties.table.depth == 1
            {
                index += 1;
            }
            if !write_table(&mut writer, &document.paragraphs[start..index])? {
                for paragraph in &document.paragraphs[start..index] {
                    write_paragraph(&mut writer, paragraph)?;
                }
            }
        } else {
            write_paragraph(&mut writer, paragraph)?;
            index += 1;
        }
    }

    write_section_properties(&mut writer, &document.section)?;

    writer.write_event(Event::End(BytesEnd::new("w:body")))?;
    writer.write_event(Event::End(BytesEnd::new("w:document")))?;
    Ok(writer.into_inner())
}

fn write_paragraph(writer: &mut Writer<Vec<u8>>, paragraph: &Paragraph) -> Result<(), OoxmlError> {
    writer.write_event(Event::Start(BytesStart::new("w:p")))?;
    write_paragraph_properties(writer, &paragraph.properties)?;
    for inline in &paragraph.children {
        match inline {
            Inline::Text(text) => write_text_run(writer, text)?,
            Inline::Symbol { font, character } => write_symbol_run(writer, font, *character)?,
            Inline::Tab => write_empty_run_element(writer, "w:tab", None)?,
            Inline::Break(kind) => {
                let break_type = match kind {
                    BreakKind::Line => None,
                    BreakKind::Page => Some("page"),
                    BreakKind::Column => Some("column"),
                };
                write_empty_run_element(writer, "w:br", break_type)?;
            }
        }
    }
    writer.write_event(Event::End(BytesEnd::new("w:p")))?;
    Ok(())
}

fn write_symbol_run(
    writer: &mut Writer<Vec<u8>>,
    font: &str,
    character: u16,
) -> Result<(), OoxmlError> {
    writer.write_event(Event::Start(BytesStart::new("w:r")))?;
    let mut symbol = BytesStart::new("w:sym");
    let code = format!("{character:04X}");
    symbol.push_attribute(("w:font", font));
    symbol.push_attribute(("w:char", code.as_str()));
    writer.write_event(Event::Empty(symbol))?;
    writer.write_event(Event::End(BytesEnd::new("w:r")))?;
    Ok(())
}

fn write_table(writer: &mut Writer<Vec<u8>>, paragraphs: &[Paragraph]) -> Result<bool, OoxmlError> {
    let mut rows: Vec<(bool, bool, Vec<Vec<&Paragraph>>)> = Vec::new();
    let mut cells: Vec<Vec<&Paragraph>> = Vec::new();
    let mut cell: Vec<&Paragraph> = Vec::new();
    for paragraph in paragraphs {
        let table = &paragraph.properties.table;
        if table.row_end {
            // The paragraph mark carrying sprmPFTtp is also the final cell's
            // paragraph mark.  It may therefore have visible text before the
            // mark; dropping it loses the entire last cell (common in RTL
            // signature tables).
            cell.push(paragraph);
            cells.push(std::mem::take(&mut cell));
            if !cells.is_empty() {
                rows.push((table.rtl, table.reverse_cells, std::mem::take(&mut cells)));
            }
            continue;
        }
        cell.push(paragraph);
        if table.cell_end {
            cells.push(std::mem::take(&mut cell));
        }
    }
    if !cell.is_empty() {
        cells.push(cell);
    }
    if !cells.is_empty() {
        rows.push((false, false, cells));
    }
    if rows.is_empty() {
        return Ok(false);
    }

    writer.write_event(Event::Start(BytesStart::new("w:tbl")))?;
    writer.write_event(Event::Start(BytesStart::new("w:tblPr")))?;
    if rows.iter().any(|(rtl, _, _)| *rtl) {
        writer.write_event(Event::Empty(BytesStart::new("w:bidiVisual")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("w:tblPr")))?;
    writer.write_event(Event::Empty(BytesStart::new("w:tblGrid")))?;
    for (_, reverse_cells, mut cells) in rows {
        if reverse_cells {
            cells.reverse();
        }
        writer.write_event(Event::Start(BytesStart::new("w:tr")))?;
        for paragraphs in cells {
            writer.write_event(Event::Start(BytesStart::new("w:tc")))?;
            writer.write_event(Event::Empty(BytesStart::new("w:tcPr")))?;
            if paragraphs.is_empty() {
                writer.write_event(Event::Empty(BytesStart::new("w:p")))?;
            } else {
                for paragraph in paragraphs {
                    write_paragraph(writer, paragraph)?;
                }
            }
            writer.write_event(Event::End(BytesEnd::new("w:tc")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("w:tr")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("w:tbl")))?;
    Ok(true)
}

fn write_paragraph_properties(
    writer: &mut Writer<Vec<u8>>,
    properties: &ParagraphProperties,
) -> Result<(), OoxmlError> {
    let has_indents = properties.indent_start.is_some()
        || properties.indent_end.is_some()
        || properties.indent_first_line.is_some();
    let has_spacing = properties.space_before.is_some() || properties.space_after.is_some();
    let has_borders = properties.borders.top.is_some()
        || properties.borders.start.is_some()
        || properties.borders.bottom.is_some()
        || properties.borders.end.is_some()
        || properties.borders.between.is_some();
    if properties.bidi.is_none()
        && properties.alignment.is_none()
        && !has_indents
        && !has_spacing
        && !has_borders
        && properties.shading.is_none()
        && properties.numbering.is_none()
    {
        return Ok(());
    }

    writer.write_event(Event::Start(BytesStart::new("w:pPr")))?;
    if properties.bidi == Some(true) {
        writer.write_event(Event::Empty(BytesStart::new("w:bidi")))?;
    }
    if let Some(alignment) = properties.alignment {
        write_value_element(writer, "w:jc", alignment_value(alignment))?;
    }
    if has_indents {
        let mut element = BytesStart::new("w:ind");
        let start;
        let end;
        let first_line;
        let hanging;
        if let Some(value) = properties.indent_start {
            start = value.to_string();
            element.push_attribute(("w:start", start.as_str()));
        }
        if let Some(value) = properties.indent_end {
            end = value.to_string();
            element.push_attribute(("w:end", end.as_str()));
        }
        if let Some(value) = properties.indent_first_line {
            if value < 0 {
                hanging = i32::from(value).unsigned_abs().to_string();
                element.push_attribute(("w:hanging", hanging.as_str()));
            } else {
                first_line = value.to_string();
                element.push_attribute(("w:firstLine", first_line.as_str()));
            }
        }
        writer.write_event(Event::Empty(element))?;
    }
    if has_spacing {
        let mut element = BytesStart::new("w:spacing");
        let before;
        let after;
        if let Some(value) = properties.space_before {
            before = value.to_string();
            element.push_attribute(("w:before", before.as_str()));
        }
        if let Some(value) = properties.space_after {
            after = value.to_string();
            element.push_attribute(("w:after", after.as_str()));
        }
        writer.write_event(Event::Empty(element))?;
    }
    if has_borders {
        writer.write_event(Event::Start(BytesStart::new("w:pBdr")))?;
        for (name, border) in [
            ("w:top", properties.borders.top.as_ref()),
            ("w:start", properties.borders.start.as_ref()),
            ("w:bottom", properties.borders.bottom.as_ref()),
            ("w:end", properties.borders.end.as_ref()),
            ("w:between", properties.borders.between.as_ref()),
        ] {
            if let Some(border) = border {
                write_border(writer, name, border)?;
            }
        }
        writer.write_event(Event::End(BytesEnd::new("w:pBdr")))?;
    }
    if let Some(shading) = &properties.shading {
        let mut element = BytesStart::new("w:shd");
        element.push_attribute(("w:val", shading.pattern.as_str()));
        if let Some(color) = &shading.foreground {
            element.push_attribute(("w:color", color.as_str()));
        }
        if let Some(fill) = &shading.background {
            element.push_attribute(("w:fill", fill.as_str()));
        }
        writer.write_event(Event::Empty(element))?;
    }
    if let Some(numbering) = properties.numbering {
        writer.write_event(Event::Start(BytesStart::new("w:numPr")))?;
        write_value_element(writer, "w:ilvl", &numbering.level.to_string())?;
        write_value_element(writer, "w:numId", &numbering.instance.to_string())?;
        writer.write_event(Event::End(BytesEnd::new("w:numPr")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("w:pPr")))?;
    Ok(())
}

fn write_border(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    border: &Border,
) -> Result<(), OoxmlError> {
    let mut element = BytesStart::new(name);
    let size = border.size.to_string();
    let space = border.space.to_string();
    element.push_attribute(("w:val", border.style.as_str()));
    element.push_attribute(("w:sz", size.as_str()));
    element.push_attribute(("w:space", space.as_str()));
    if let Some(color) = &border.color {
        element.push_attribute(("w:color", color.as_str()));
    }
    if border.shadow {
        element.push_attribute(("w:shadow", "1"));
    }
    if border.frame {
        element.push_attribute(("w:frame", "1"));
    }
    writer.write_event(Event::Empty(element))?;
    Ok(())
}

fn numbering_xml(document: &Document) -> Result<Vec<u8>, OoxmlError> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        Some("yes"),
    )))?;
    let mut root = BytesStart::new("w:numbering");
    root.push_attribute((
        "xmlns:w",
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    ));
    writer.write_event(Event::Start(root))?;
    for definition in &document.numbering {
        let abstract_id = definition.instance.to_string();
        let mut abstract_num = BytesStart::new("w:abstractNum");
        abstract_num.push_attribute(("w:abstractNumId", abstract_id.as_str()));
        writer.write_event(Event::Start(abstract_num))?;
        write_value_element(&mut writer, "w:multiLevelType", "hybridMultilevel")?;
        for level in &definition.levels {
            write_numbering_level(&mut writer, level)?;
        }
        writer.write_event(Event::End(BytesEnd::new("w:abstractNum")))?;
    }
    for definition in &document.numbering {
        let id = definition.instance.to_string();
        let mut num = BytesStart::new("w:num");
        num.push_attribute(("w:numId", id.as_str()));
        writer.write_event(Event::Start(num))?;
        write_value_element(&mut writer, "w:abstractNumId", &id)?;
        for override_level in &definition.overrides {
            let level_id = override_level.level.to_string();
            let mut element = BytesStart::new("w:lvlOverride");
            element.push_attribute(("w:ilvl", level_id.as_str()));
            writer.write_event(Event::Start(element))?;
            if let Some(start) = override_level.start {
                write_value_element(&mut writer, "w:startOverride", &start.to_string())?;
            }
            if let Some(formatting) = &override_level.formatting {
                write_numbering_level(&mut writer, formatting)?;
            }
            writer.write_event(Event::End(BytesEnd::new("w:lvlOverride")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("w:num")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("w:numbering")))?;
    Ok(writer.into_inner())
}

fn write_numbering_level(
    writer: &mut Writer<Vec<u8>>,
    level: &docxthedocs_ir::NumberingLevel,
) -> Result<(), OoxmlError> {
    let level_id = level.level.to_string();
    let mut level_start = BytesStart::new("w:lvl");
    level_start.push_attribute(("w:ilvl", level_id.as_str()));
    writer.write_event(Event::Start(level_start))?;
    write_value_element(writer, "w:start", &level.start.to_string())?;
    write_value_element(writer, "w:numFmt", &level.format)?;
    write_value_element(writer, "w:lvlText", &level.text)?;
    write_value_element(writer, "w:lvlJc", alignment_value(level.alignment))?;
    let suffix = match level.suffix {
        NumberingSuffix::Tab => "tab",
        NumberingSuffix::Space => "space",
        NumberingSuffix::Nothing => "nothing",
    };
    write_value_element(writer, "w:suff", suffix)?;
    write_paragraph_properties(writer, &level.paragraph)?;
    writer.write_event(Event::End(BytesEnd::new("w:lvl")))?;
    Ok(())
}

fn alignment_value(alignment: Alignment) -> &'static str {
    match alignment {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
        Alignment::Both => "both",
        Alignment::Distribute => "distribute",
        Alignment::MediumKashida => "mediumKashida",
        Alignment::HighKashida => "highKashida",
        Alignment::LowKashida => "lowKashida",
        Alignment::ThaiDistribute => "thaiDistribute",
    }
}

fn write_section_properties(
    writer: &mut Writer<Vec<u8>>,
    properties: &SectionProperties,
) -> Result<(), OoxmlError> {
    let has_page_size = properties.page_width.is_some()
        || properties.page_height.is_some()
        || properties.landscape.is_some();
    let has_margins = properties.margin_top.is_some()
        || properties.margin_bottom.is_some()
        || properties.margin_left.is_some()
        || properties.margin_right.is_some();
    if !has_page_size
        && !has_margins
        && properties.bidi.is_none()
        && properties.rtl_gutter.is_none()
    {
        return Ok(());
    }

    writer.write_event(Event::Start(BytesStart::new("w:sectPr")))?;
    if has_page_size {
        let mut element = BytesStart::new("w:pgSz");
        let width;
        let height;
        if let Some(value) = properties.page_width {
            width = value.to_string();
            element.push_attribute(("w:w", width.as_str()));
        }
        if let Some(value) = properties.page_height {
            height = value.to_string();
            element.push_attribute(("w:h", height.as_str()));
        }
        if properties.landscape == Some(true) {
            element.push_attribute(("w:orient", "landscape"));
        }
        writer.write_event(Event::Empty(element))?;
    }
    if has_margins {
        let mut element = BytesStart::new("w:pgMar");
        let top;
        let bottom;
        let left;
        let right;
        if let Some(value) = properties.margin_top {
            top = value.to_string();
            element.push_attribute(("w:top", top.as_str()));
        }
        if let Some(value) = properties.margin_bottom {
            bottom = value.to_string();
            element.push_attribute(("w:bottom", bottom.as_str()));
        }
        if let Some(value) = properties.margin_left {
            left = value.to_string();
            element.push_attribute(("w:left", left.as_str()));
        }
        if let Some(value) = properties.margin_right {
            right = value.to_string();
            element.push_attribute(("w:right", right.as_str()));
        }
        writer.write_event(Event::Empty(element))?;
    }
    if properties.bidi == Some(true) {
        writer.write_event(Event::Empty(BytesStart::new("w:bidi")))?;
    }
    if properties.rtl_gutter == Some(true) {
        writer.write_event(Event::Empty(BytesStart::new("w:rtlGutter")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("w:sectPr")))?;
    Ok(())
}

fn write_value_element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    value: &str,
) -> Result<(), OoxmlError> {
    let mut element = BytesStart::new(name);
    element.push_attribute(("w:val", value));
    writer.write_event(Event::Empty(element))?;
    Ok(())
}

fn write_text_run(writer: &mut Writer<Vec<u8>>, text: &str) -> Result<(), OoxmlError> {
    writer.write_event(Event::Start(BytesStart::new("w:r")))?;
    let mut text_start = BytesStart::new("w:t");
    if text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.contains("  ")
    {
        text_start.push_attribute(("xml:space", "preserve"));
    }
    writer.write_event(Event::Start(text_start))?;
    writer.write_event(Event::Text(BytesText::new(text)))?;
    writer.write_event(Event::End(BytesEnd::new("w:t")))?;
    writer.write_event(Event::End(BytesEnd::new("w:r")))?;
    Ok(())
}

fn write_empty_run_element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    break_type: Option<&str>,
) -> Result<(), OoxmlError> {
    writer.write_event(Event::Start(BytesStart::new("w:r")))?;
    let mut element = BytesStart::new(name);
    if let Some(value) = break_type {
        element.push_attribute(("w:type", value));
    }
    writer.write_event(Event::Empty(element))?;
    writer.write_event(Event::End(BytesEnd::new("w:r")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use docxthedocs_ir::{Inline, Paragraph};

    #[test]
    fn escapes_text_and_preserves_whitespace() {
        let document = Document {
            paragraphs: vec![Paragraph {
                properties: ParagraphProperties::default(),
                children: vec![Inline::Text(" א<&ב ".to_owned())],
            }],
            section: SectionProperties::default(),
            numbering: Vec::new(),
        };
        let xml = String::from_utf8(document_xml(&document).unwrap()).unwrap();
        assert!(xml.contains("xml:space=\"preserve\""));
        assert!(xml.contains(" א&lt;&amp;ב "));
    }

    #[test]
    fn writes_rtl_alignment_indents_and_page_margins() {
        let document = Document {
            paragraphs: vec![Paragraph {
                properties: ParagraphProperties {
                    bidi: Some(true),
                    alignment: Some(Alignment::Right),
                    indent_start: Some(720),
                    indent_end: Some(360),
                    indent_first_line: Some(-240),
                    space_before: Some(120),
                    space_after: Some(80),
                    numbering: None,
                    borders: Default::default(),
                    shading: None,
                    table: Default::default(),
                },
                children: vec![Inline::Text("עברית".to_owned())],
            }],
            section: SectionProperties {
                page_width: Some(16_839),
                page_height: Some(11_907),
                margin_top: Some(1_440),
                margin_bottom: Some(1_440),
                margin_left: Some(1_800),
                margin_right: Some(1_800),
                landscape: Some(true),
                bidi: Some(true),
                rtl_gutter: Some(true),
            },
            numbering: Vec::new(),
        };
        let xml = String::from_utf8(document_xml(&document).unwrap()).unwrap();
        assert!(xml.contains("<w:bidi/>") && xml.contains("w:val=\"right\""));
        assert!(xml.contains("w:start=\"720\"") && xml.contains("w:hanging=\"240\""));
        assert!(xml.contains("<w:pgSz w:w=\"16839\" w:h=\"11907\" w:orient=\"landscape\"/>"));
        assert!(xml.contains(
            "<w:pgMar w:top=\"1440\" w:bottom=\"1440\" w:left=\"1800\" w:right=\"1800\"/>"
        ));
        assert!(xml.contains("<w:rtlGutter/>"));
    }
}
