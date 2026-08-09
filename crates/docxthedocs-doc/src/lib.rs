use docxthedocs_ir::{
    Alignment, Border, BreakKind, Document, Inline, NumberingDefinition, NumberingLevel,
    NumberingOverride, NumberingRef, NumberingSuffix, Paragraph, ParagraphProperties,
    SectionProperties, Shading, StoryCounts,
};
use std::char::decode_utf16;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const FIB_BASE_SIZE: usize = 32;
const FIB_CSW_OFFSET: usize = 32;
const FIB_CSLW_OFFSET: usize = 62;
const FIB_LW_OFFSET: usize = 64;
const FIB_CB_RG_FC_LCB_OFFSET: usize = 152;
const FIB_FC_LCB_OFFSET: usize = 154;
const FC_CLX_PAIR_INDEX: usize = 33;
const FC_PLCF_SED_PAIR_INDEX: usize = 6;
const FC_PLCF_BTE_CHPX_PAIR_INDEX: usize = 12;
const FC_PLCF_BTE_PAPX_PAIR_INDEX: usize = 13;
const FC_PLC_SPA_MOM_PAIR_INDEX: usize = 40;
const FC_PLCF_TXBX_TXT_PAIR_INDEX: usize = 56;
const FC_PLF_LST_PAIR_INDEX: usize = 73;
const FC_PLF_LFO_PAIR_INDEX: usize = 74;
const MAX_PIECES: usize = 1_000_000;
const MAX_CHARACTER_POSITIONS: u32 = 100_000_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DocError {
    #[error("WordDocument stream is truncated while reading {field}")]
    TruncatedFib { field: &'static str },
    #[error("invalid Word binary signature 0x{0:04X}; expected 0xA5EC")]
    InvalidSignature(u16),
    #[error("invalid FIB field {field}: {value}")]
    InvalidFibField { field: &'static str, value: i64 },
    #[error("encrypted or obfuscated Word documents are not supported")]
    Encrypted,
    #[error("{table} stream range for CLX is outside the stream")]
    ClxOutOfBounds { table: &'static str },
    #[error("CLX is malformed: {0}")]
    MalformedClx(String),
    #[error("piece table contains too many pieces ({actual}; limit {limit})")]
    TooManyPieces { actual: usize, limit: usize },
    #[error("piece {piece} points outside the WordDocument stream")]
    PieceOutOfBounds { piece: usize },
    #[error("piece {piece} contains invalid UTF-16")]
    InvalidUtf16 { piece: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fib {
    pub n_fib: u16,
    pub table_stream: &'static str,
    pub f_complex: bool,
    pub f_has_pic: bool,
    pub cb_mac: u32,
    pub fc_clx: u32,
    pub lcb_clx: u32,
    pub fc_plcf_sed: u32,
    pub lcb_plcf_sed: u32,
    pub fc_plcf_bte_chpx: u32,
    pub lcb_plcf_bte_chpx: u32,
    pub fc_plcf_bte_papx: u32,
    pub lcb_plcf_bte_papx: u32,
    pub fc_plc_spa_mom: u32,
    pub lcb_plc_spa_mom: u32,
    pub fc_plcf_txbx_txt: u32,
    pub lcb_plcf_txbx_txt: u32,
    pub fc_plf_lst: u32,
    pub lcb_plf_lst: u32,
    pub fc_plf_lfo: u32,
    pub lcb_plf_lfo: u32,
    pub stories: StoryCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOutput {
    pub document: Document,
    pub fib: Fib,
    pub features_seen: BTreeSet<String>,
    pub unsupported: BTreeSet<String>,
    pub warnings: Vec<String>,
    pub piece_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    cp_start: u32,
    cp_end: u32,
    raw_fc: u32,
}

pub fn parse_fib(word_document: &[u8]) -> Result<Fib, DocError> {
    require_len(word_document, FIB_BASE_SIZE, "FibBase")?;
    let signature = read_u16(word_document, 0, "wIdent")?;
    if signature != 0xA5EC {
        return Err(DocError::InvalidSignature(signature));
    }
    let n_fib = read_u16(word_document, 2, "nFib")?;
    if n_fib < 0x00C1 {
        return Err(DocError::InvalidFibField {
            field: "nFib",
            value: i64::from(n_fib),
        });
    }

    let flags = read_u16(word_document, 10, "FibBase flags")?;
    let f_complex = flags & (1 << 2) != 0;
    let f_has_pic = flags & (1 << 3) != 0;
    let encrypted = flags & (1 << 8) != 0;
    let table_stream = if flags & (1 << 9) != 0 {
        "1Table"
    } else {
        "0Table"
    };
    if encrypted {
        return Err(DocError::Encrypted);
    }

    let csw = read_u16(word_document, FIB_CSW_OFFSET, "csw")?;
    if csw != 0x000E {
        return Err(DocError::InvalidFibField {
            field: "csw",
            value: i64::from(csw),
        });
    }
    let cslw = read_u16(word_document, FIB_CSLW_OFFSET, "cslw")?;
    if cslw != 0x0016 {
        return Err(DocError::InvalidFibField {
            field: "cslw",
            value: i64::from(cslw),
        });
    }

    let cb_mac = read_u32(word_document, FIB_LW_OFFSET, "cbMac")?;
    let stories = StoryCounts {
        main: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 12, "ccpText")?,
        footnotes: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 16, "ccpFtn")?,
        headers: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 20, "ccpHdd")?,
        comments: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 28, "ccpAtn")?,
        endnotes: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 32, "ccpEdn")?,
        textboxes: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 36, "ccpTxbx")?,
        header_textboxes: read_nonnegative_i32(word_document, FIB_LW_OFFSET + 40, "ccpHdrTxbx")?,
    };
    let total = stories.total().ok_or(DocError::InvalidFibField {
        field: "story character total",
        value: -1,
    })?;
    if stories.main == 0 || total > MAX_CHARACTER_POSITIONS {
        return Err(DocError::InvalidFibField {
            field: "story character total",
            value: i64::from(total),
        });
    }

    let cb_rg_fc_lcb = read_u16(word_document, FIB_CB_RG_FC_LCB_OFFSET, "cbRgFcLcb")? as usize;
    if cb_rg_fc_lcb <= FC_CLX_PAIR_INDEX {
        return Err(DocError::InvalidFibField {
            field: "cbRgFcLcb",
            value: cb_rg_fc_lcb as i64,
        });
    }
    let (fc_clx, lcb_clx) = read_fc_lcb_pair(word_document, FC_CLX_PAIR_INDEX, "fcClx")?;
    if lcb_clx == 0 {
        return Err(DocError::InvalidFibField {
            field: "lcbClx",
            value: 0,
        });
    }

    let (fc_plcf_sed, lcb_plcf_sed) =
        read_fc_lcb_pair(word_document, FC_PLCF_SED_PAIR_INDEX, "fcPlcfSed")?;
    let (fc_plcf_bte_chpx, lcb_plcf_bte_chpx) =
        read_fc_lcb_pair(word_document, FC_PLCF_BTE_CHPX_PAIR_INDEX, "fcPlcfBteChpx")?;
    let (fc_plcf_bte_papx, lcb_plcf_bte_papx) =
        read_fc_lcb_pair(word_document, FC_PLCF_BTE_PAPX_PAIR_INDEX, "fcPlcfBtePapx")?;
    let (fc_plc_spa_mom, lcb_plc_spa_mom) =
        read_fc_lcb_pair(word_document, FC_PLC_SPA_MOM_PAIR_INDEX, "fcPlcSpaMom")?;
    let (fc_plcf_txbx_txt, lcb_plcf_txbx_txt) =
        read_fc_lcb_pair(word_document, FC_PLCF_TXBX_TXT_PAIR_INDEX, "fcPlcftxbxTxt")?;
    let (fc_plf_lst, lcb_plf_lst) =
        read_fc_lcb_pair(word_document, FC_PLF_LST_PAIR_INDEX, "fcPlfLst")?;
    let (fc_plf_lfo, lcb_plf_lfo) =
        read_fc_lcb_pair(word_document, FC_PLF_LFO_PAIR_INDEX, "fcPlfLfo")?;

    Ok(Fib {
        n_fib,
        table_stream,
        f_complex,
        f_has_pic,
        cb_mac,
        fc_clx,
        lcb_clx,
        fc_plcf_sed,
        lcb_plcf_sed,
        fc_plcf_bte_chpx,
        lcb_plcf_bte_chpx,
        fc_plcf_bte_papx,
        lcb_plcf_bte_papx,
        fc_plc_spa_mom,
        lcb_plc_spa_mom,
        fc_plcf_txbx_txt,
        lcb_plcf_txbx_txt,
        fc_plf_lst,
        lcb_plf_lst,
        fc_plf_lfo,
        lcb_plf_lfo,
        stories,
    })
}

pub fn parse_document(
    word_document: &[u8],
    table_stream: &[u8],
    data_stream: &[u8],
    fib: Fib,
) -> Result<ParseOutput, DocError> {
    let clx_start = usize::try_from(fib.fc_clx).map_err(|_| DocError::ClxOutOfBounds {
        table: fib.table_stream,
    })?;
    let clx_len = usize::try_from(fib.lcb_clx).map_err(|_| DocError::ClxOutOfBounds {
        table: fib.table_stream,
    })?;
    let clx_end = clx_start
        .checked_add(clx_len)
        .filter(|end| *end <= table_stream.len())
        .ok_or(DocError::ClxOutOfBounds {
            table: fib.table_stream,
        })?;
    let pieces = parse_clx(&table_stream[clx_start..clx_end], &fib)?;
    let mut text = reconstruct_main_text(word_document, &pieces, fib.stories.main)?;

    let mut features_seen = BTreeSet::new();
    let mut unsupported = BTreeSet::from(["styles:not-resolved".to_owned()]);
    let mut warnings = vec![
        "style-inherited formatting is not resolved; direct paragraph and section properties are preserved"
            .to_owned(),
    ];
    features_seen.insert("main-text".to_owned());
    if fib.f_complex {
        features_seen.insert("fast-saved".to_owned());
    }
    if fib.f_has_pic {
        features_seen.insert("pictures".to_owned());
        unsupported.insert("pictures".to_owned());
    }
    add_story_capabilities(&fib.stories, &mut features_seen, &mut unsupported);

    if fib.lcb_plcf_bte_chpx != 0 {
        match apply_hidden_character_formatting(
            word_document,
            table_stream,
            &fib,
            &pieces,
            &mut text,
        ) {
            Ok((hidden_count, deleted_count)) if hidden_count != 0 || deleted_count != 0 => {
                if hidden_count != 0 {
                    features_seen.insert("hidden-text".to_owned());
                    warnings.push(format!(
                        "suppressed {hidden_count} characters formatted as hidden"
                    ));
                }
                if deleted_count != 0 {
                    features_seen.insert("revision-deletions".to_owned());
                    warnings.push(format!(
                        "suppressed {deleted_count} characters marked as deleted revisions"
                    ));
                }
            }
            Ok(_) => {}
            Err(error) => {
                unsupported.insert("character-formatting:partial".to_owned());
                warnings.push(format!(
                    "hidden character formatting was not recovered: {error}"
                ));
            }
        }
    }

    let symbols = if fib.lcb_plcf_bte_chpx != 0 {
        match collect_symbol_character_formatting(
            word_document,
            table_stream,
            &fib,
            &pieces,
            text.len(),
        ) {
            Ok(symbols) => {
                let count = symbols.iter().filter(|symbol| symbol.is_some()).count();
                if count != 0 {
                    features_seen.insert("symbol-glyphs".to_owned());
                    warnings.push(format!("preserved {count} custom symbol glyphs"));
                }
                symbols
            }
            Err(error) => {
                unsupported.insert("character-symbols:partial".to_owned());
                warnings.push(format!(
                    "custom symbol formatting was not recovered: {error}"
                ));
                vec![None; text.len()]
            }
        }
    } else {
        vec![None; text.len()]
    };

    let textboxes =
        parse_textboxes(word_document, table_stream, &fib, &pieces).unwrap_or_else(|error| {
            warnings.push(format!("textbox text was not recovered: {error}"));
            TextboxPlacements::default()
        });
    if textboxes.unplaced != 0 {
        warnings.push(format!(
            "{} textbox stories belong to grouped OfficeArt shapes and were not placed",
            textboxes.unplaced
        ));
    }
    let (mut document, paragraph_cps) = text_to_ir(
        &text,
        &symbols,
        &textboxes,
        &mut features_seen,
        &mut unsupported,
        &mut warnings,
    );
    if fib.lcb_plcf_bte_papx != 0 {
        let formatting = FormattingContext {
            word_document,
            table_stream,
            data_stream,
            fib: &fib,
            pieces: &pieces,
            text: &text,
        };
        match apply_paragraph_formatting(formatting, &paragraph_cps, &mut document) {
            Ok(count) => {
                features_seen.insert("paragraph-formatting".to_owned());
                if document.paragraphs.iter().any(|paragraph| {
                    let properties = &paragraph.properties;
                    properties.shading.is_some()
                        || properties.borders.top.is_some()
                        || properties.borders.start.is_some()
                        || properties.borders.bottom.is_some()
                        || properties.borders.end.is_some()
                        || properties.borders.between.is_some()
                }) {
                    features_seen.insert("simple-graphics".to_owned());
                }
                if count != document.paragraphs.len() {
                    unsupported.insert("paragraph-formatting:partial".to_owned());
                    warnings.push(format!(
                        "direct paragraph properties were recovered for {count} of {} paragraphs",
                        document.paragraphs.len()
                    ));
                }
            }
            Err(error) => {
                unsupported.insert("paragraph-formatting:malformed".to_owned());
                warnings.push(format!("paragraph formatting was not recovered: {error}"));
            }
        }
    }
    let inferred_rtl = infer_rtl_paragraphs(&mut document);
    if inferred_rtl != 0 {
        features_seen.insert("rtl-inferred-from-text".to_owned());
        warnings.push(format!(
            "inferred RTL direction for {inferred_rtl} Hebrew/Arabic paragraphs without a direct bidi property"
        ));
    }
    if fib.lcb_plcf_sed != 0 {
        match parse_section_properties(word_document, table_stream, &fib) {
            Ok((section, section_count)) => {
                document.section = section;
                features_seen.insert("page-layout".to_owned());
                if section_count > 1 {
                    unsupported.insert("sections:multiple".to_owned());
                    warnings.push(format!(
                        "document has {section_count} sections; the final section layout is currently applied to the DOCX body"
                    ));
                }
            }
            Err(error) => {
                unsupported.insert("page-layout:malformed".to_owned());
                warnings.push(format!("section properties were not recovered: {error}"));
            }
        }
    }
    if fib.lcb_plf_lst != 0 || fib.lcb_plf_lfo != 0 {
        features_seen.insert("automatic-numbering".to_owned());
        match parse_numbering(table_stream, &fib) {
            Ok(definitions) => {
                document.numbering = definitions;
                if document.numbering.is_empty() {
                    unsupported.insert("automatic-numbering:empty".to_owned());
                }
            }
            Err(error) => {
                unsupported.insert("automatic-numbering:malformed".to_owned());
                warnings.push(format!("automatic numbering was not recovered: {error}"));
            }
        }
    }
    Ok(ParseOutput {
        document,
        fib,
        features_seen,
        unsupported,
        warnings,
        piece_count: pieces.len(),
    })
}

fn apply_hidden_character_formatting(
    word_document: &[u8],
    table_stream: &[u8],
    fib: &Fib,
    pieces: &[Piece],
    text: &mut [char],
) -> Result<(usize, usize), String> {
    let plc = checked_range(
        table_stream,
        fib.fc_plcf_bte_chpx,
        fib.lcb_plcf_bte_chpx,
        "PlcBteChpx",
    )?;
    if plc.len() < 12 || (plc.len() - 4) % 8 != 0 {
        return Err(format!("invalid PlcBteChpx length {}", plc.len()));
    }
    let page_count = (plc.len() - 4) / 8;
    let fc_count = page_count + 1;
    let page_numbers_offset = fc_count * 4;
    let mut hidden = vec![false; text.len()];
    let mut deleted = vec![false; text.len()];

    for page_index in 0..page_count {
        let pn_raw = read_u32_at(plc, page_numbers_offset + page_index * 4, "PnFkpChpx")?;
        let page_offset = usize::try_from(pn_raw & 0x003F_FFFF)
            .ok()
            .and_then(|pn| pn.checked_mul(512))
            .ok_or_else(|| "ChpxFkp offset overflowed".to_owned())?;
        let page = word_document
            .get(page_offset..page_offset + 512)
            .ok_or_else(|| format!("ChpxFkp at {page_offset} is outside WordDocument"))?;
        let run_count = usize::from(page[511]);
        if !(1..=0x65).contains(&run_count) {
            return Err(format!("ChpxFkp has invalid crun {run_count}"));
        }
        let rgb_offset = (run_count + 1) * 4;
        for run_index in 0..run_count {
            let chpx_offset = usize::from(page[rgb_offset + run_index]) * 2;
            if chpx_offset == 0 {
                continue;
            }
            let length = usize::from(
                *page
                    .get(chpx_offset)
                    .ok_or_else(|| "Chpx.cb is outside page".to_owned())?,
            );
            let grpprl = page
                .get(chpx_offset + 1..chpx_offset + 1 + length)
                .ok_or_else(|| "Chpx.grpprl is outside page".to_owned())?;
            let mut vanish = false;
            let mut revision_deleted = false;
            for_each_prl(grpprl, |sprm, operand| {
                if sprm == 0x083C && !operand.is_empty() {
                    vanish = operand[0] & 1 != 0;
                } else if sprm == 0x0800 && !operand.is_empty() {
                    revision_deleted = operand[0] & 1 != 0;
                }
                Ok(())
            })?;
            if !vanish && !revision_deleted {
                continue;
            }
            let fc_start = read_u32_at(page, run_index * 4, "ChpxFkp.rgfc")?;
            let fc_end = read_u32_at(page, (run_index + 1) * 4, "ChpxFkp.rgfc")?;
            if vanish {
                mark_character_fc_range(pieces, fc_start, fc_end, &mut hidden);
            }
            if revision_deleted {
                mark_character_fc_range(pieces, fc_start, fc_end, &mut deleted);
            }
        }
    }

    let mut hidden_count = 0_usize;
    let mut deleted_count = 0_usize;
    for (index, character) in text.iter_mut().enumerate() {
        let is_hidden = hidden[index];
        let is_deleted = deleted[index];
        if is_hidden || is_deleted {
            if *character == '\r' {
                // Word joins paragraphs across vanished or revision-deleted
                // paragraph marks. Keep the visible boundary as a line break
                // without advancing automatic numbering.
                *character = '\u{000B}';
                hidden_count += usize::from(is_hidden);
                deleted_count += usize::from(is_deleted);
            } else if !is_structural_character(*character) {
                *character = '\u{FFFC}';
                hidden_count += usize::from(is_hidden);
                deleted_count += usize::from(is_deleted);
            }
        }
    }
    Ok((hidden_count, deleted_count))
}

fn collect_symbol_character_formatting(
    word_document: &[u8],
    table_stream: &[u8],
    fib: &Fib,
    pieces: &[Piece],
    text_len: usize,
) -> Result<Vec<Option<u16>>, String> {
    let plc = checked_range(
        table_stream,
        fib.fc_plcf_bte_chpx,
        fib.lcb_plcf_bte_chpx,
        "PlcBteChpx",
    )?;
    if plc.len() < 12 || (plc.len() - 4) % 8 != 0 {
        return Err(format!("invalid PlcBteChpx length {}", plc.len()));
    }
    let page_count = (plc.len() - 4) / 8;
    let fc_count = page_count + 1;
    let page_numbers_offset = fc_count * 4;
    let mut symbols = vec![None; text_len];

    for page_index in 0..page_count {
        let pn_raw = read_u32_at(plc, page_numbers_offset + page_index * 4, "PnFkpChpx")?;
        let page_offset = usize::try_from(pn_raw & 0x003F_FFFF)
            .ok()
            .and_then(|pn| pn.checked_mul(512))
            .ok_or_else(|| "ChpxFkp offset overflowed".to_owned())?;
        let page = word_document
            .get(page_offset..page_offset + 512)
            .ok_or_else(|| format!("ChpxFkp at {page_offset} is outside WordDocument"))?;
        let run_count = usize::from(page[511]);
        if !(1..=0x65).contains(&run_count) {
            return Err(format!("ChpxFkp has invalid crun {run_count}"));
        }
        let rgb_offset = (run_count + 1) * 4;
        for run_index in 0..run_count {
            let chpx_offset = usize::from(page[rgb_offset + run_index]) * 2;
            if chpx_offset == 0 {
                continue;
            }
            let length = usize::from(
                *page
                    .get(chpx_offset)
                    .ok_or_else(|| "Chpx.cb is outside page".to_owned())?,
            );
            let grpprl = page
                .get(chpx_offset + 1..chpx_offset + 1 + length)
                .ok_or_else(|| "Chpx.grpprl is outside page".to_owned())?;
            let mut symbol = None;
            for_each_prl(grpprl, |sprm, operand| {
                // sprmCSymbol: ftcSym (2 bytes), xchSym (2 bytes).
                if sprm == 0x6A09 && operand.len() >= 4 {
                    symbol = Some(u16_from(&operand[2..4]));
                }
                Ok(())
            })?;
            let Some(symbol) = symbol else {
                continue;
            };
            let fc_start = read_u32_at(page, run_index * 4, "ChpxFkp.rgfc")?;
            let fc_end = read_u32_at(page, (run_index + 1) * 4, "ChpxFkp.rgfc")?;
            mark_symbol_fc_range(pieces, fc_start, fc_end, symbol, &mut symbols);
        }
    }
    Ok(symbols)
}

fn mark_symbol_fc_range(
    pieces: &[Piece],
    fc_start: u32,
    fc_end: u32,
    symbol: u16,
    symbols: &mut [Option<u16>],
) {
    for piece in pieces {
        let compressed = piece.raw_fc & 0x4000_0000 != 0;
        let encoded_fc = piece.raw_fc & 0x3FFF_FFFF;
        let piece_fc = if compressed {
            encoded_fc / 2
        } else {
            encoded_fc
        };
        let bytes_per_cp = if compressed { 1 } else { 2 };
        let Some(piece_fc_end) = piece_fc.checked_add(
            piece
                .cp_end
                .saturating_sub(piece.cp_start)
                .saturating_mul(bytes_per_cp),
        ) else {
            continue;
        };
        let start = fc_start.max(piece_fc);
        let end = fc_end.min(piece_fc_end);
        if start >= end {
            continue;
        }
        let cp_start = piece.cp_start + (start - piece_fc) / bytes_per_cp;
        let cp_end = piece.cp_start + (end - piece_fc).div_ceil(bytes_per_cp);
        for cp in cp_start..cp_end.min(piece.cp_end) {
            if let Some(slot) = symbols.get_mut(cp as usize) {
                *slot = Some(symbol);
            }
        }
    }
}

fn mark_character_fc_range(pieces: &[Piece], fc_start: u32, fc_end: u32, marked: &mut [bool]) {
    for piece in pieces {
        let compressed = piece.raw_fc & 0x4000_0000 != 0;
        let encoded_fc = piece.raw_fc & 0x3FFF_FFFF;
        let piece_fc = if compressed {
            encoded_fc / 2
        } else {
            encoded_fc
        };
        let bytes_per_cp = if compressed { 1 } else { 2 };
        let Some(piece_fc_end) = piece_fc.checked_add(
            piece
                .cp_end
                .saturating_sub(piece.cp_start)
                .saturating_mul(bytes_per_cp),
        ) else {
            continue;
        };
        let start = fc_start.max(piece_fc);
        let end = fc_end.min(piece_fc_end);
        if start >= end {
            continue;
        }
        let cp_start = piece.cp_start + (start - piece_fc) / bytes_per_cp;
        let cp_end = piece.cp_start + (end - piece_fc).div_ceil(bytes_per_cp);
        for cp in cp_start..cp_end.min(piece.cp_end) {
            if let Some(slot) = marked.get_mut(cp as usize) {
                *slot = true;
            }
        }
    }
}

fn is_structural_character(character: char) -> bool {
    matches!(
        character,
        '\r' | '\u{0007}' | '\u{0013}' | '\u{0014}' | '\u{0015}'
    )
}

fn infer_rtl_paragraphs(document: &mut Document) -> usize {
    let mut inferred = 0_usize;
    for paragraph in &mut document.paragraphs {
        if paragraph.properties.bidi.is_some() {
            continue;
        }
        let mut rtl = 0_usize;
        let mut ltr = 0_usize;
        for ch in paragraph
            .children
            .iter()
            .filter_map(|inline| match inline {
                Inline::Text(text) => Some(text.chars()),
                _ => None,
            })
            .flatten()
        {
            match ch as u32 {
                0x0590..=0x08FF => rtl += 1,
                0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x02AF => ltr += 1,
                _ => {}
            }
        }
        if rtl > ltr && rtl != 0 {
            paragraph.properties.bidi = Some(true);
            inferred += 1;
        }
    }
    inferred
}

fn parse_clx(clx: &[u8], fib: &Fib) -> Result<Vec<Piece>, DocError> {
    let mut cursor = 0_usize;
    while cursor < clx.len() && clx[cursor] == 0x01 {
        let len_offset = cursor + 1;
        let grpprl_len = usize::from(read_u16_slice(clx, len_offset, "Prc.cbGrpprl")?);
        if grpprl_len > 0x3FA2 {
            return Err(DocError::MalformedClx(format!(
                "Prc.grpprl is too large ({grpprl_len})"
            )));
        }
        cursor = len_offset
            .checked_add(2)
            .and_then(|value| value.checked_add(grpprl_len))
            .filter(|value| *value <= clx.len())
            .ok_or_else(|| DocError::MalformedClx("truncated Prc".to_owned()))?;
    }
    if clx.get(cursor) != Some(&0x02) {
        return Err(DocError::MalformedClx(
            "missing Pcdt marker 0x02".to_owned(),
        ));
    }
    let lcb = usize::try_from(read_u32_slice(clx, cursor + 1, "Pcdt.lcb")?)
        .map_err(|_| DocError::MalformedClx("Pcdt.lcb is too large".to_owned()))?;
    let plc_start = cursor + 5;
    let plc_end = plc_start
        .checked_add(lcb)
        .filter(|end| *end <= clx.len())
        .ok_or_else(|| DocError::MalformedClx("truncated PlcPcd".to_owned()))?;
    if lcb < 4 || (lcb - 4) % 12 != 0 {
        return Err(DocError::MalformedClx(format!(
            "PlcPcd length {lcb} does not encode whole Pcd records"
        )));
    }
    let piece_count = (lcb - 4) / 12;
    if piece_count == 0 {
        return Err(DocError::MalformedClx("empty piece table".to_owned()));
    }
    if piece_count > MAX_PIECES {
        return Err(DocError::TooManyPieces {
            actual: piece_count,
            limit: MAX_PIECES,
        });
    }
    let plc = &clx[plc_start..plc_end];
    let cp_bytes = (piece_count + 1) * 4;
    let mut cps = Vec::with_capacity(piece_count + 1);
    for index in 0..=piece_count {
        let cp = read_i32_slice(plc, index * 4, "PlcPcd.aCP")?;
        if cp < 0 {
            return Err(DocError::MalformedClx(format!(
                "negative CP at index {index}"
            )));
        }
        let cp = cp as u32;
        if index > 0 && cp <= cps[index - 1] {
            return Err(DocError::MalformedClx(format!(
                "CP array is not strictly increasing at index {index}"
            )));
        }
        cps.push(cp);
    }

    let total_story_cp = fib
        .stories
        .total()
        .ok_or_else(|| DocError::MalformedClx("story character total overflowed".to_owned()))?;
    let has_subdocuments = total_story_cp != fib.stories.main;
    let expected_last = if has_subdocuments {
        total_story_cp
            .checked_add(1)
            .ok_or_else(|| DocError::MalformedClx("piece-table CP total overflowed".to_owned()))?
    } else {
        fib.stories.main
    };
    if cps.last().copied() != Some(expected_last) {
        return Err(DocError::MalformedClx(format!(
            "last CP is {}, expected {expected_last}",
            cps.last().copied().unwrap_or_default()
        )));
    }

    let mut pieces = Vec::with_capacity(piece_count);
    for index in 0..piece_count {
        let pcd_offset = cp_bytes + index * 8;
        let raw_fc = read_u32_slice(plc, pcd_offset + 2, "Pcd.fc")?;
        pieces.push(Piece {
            cp_start: cps[index],
            cp_end: cps[index + 1],
            raw_fc,
        });
    }
    Ok(pieces)
}

fn reconstruct_main_text(
    word_document: &[u8],
    pieces: &[Piece],
    main_cp: u32,
) -> Result<Vec<char>, DocError> {
    let mut out = Vec::with_capacity(main_cp as usize);
    for (index, piece) in pieces.iter().enumerate() {
        let cp_start = piece.cp_start.min(main_cp);
        let cp_end = piece.cp_end.min(main_cp);
        if cp_start >= cp_end {
            continue;
        }
        let skip_cp = cp_start - piece.cp_start;
        let take_cp = cp_end - cp_start;
        let compressed = piece.raw_fc & 0x4000_0000 != 0;
        let fc = piece.raw_fc & 0x3FFF_FFFF;
        if piece.raw_fc & 0x8000_0000 != 0 {
            return Err(DocError::MalformedClx(format!(
                "Pcd {index} has reserved FcCompressed bit set"
            )));
        }

        if compressed {
            if fc % 2 != 0 {
                return Err(DocError::MalformedClx(format!(
                    "compressed Pcd {index} has an odd encoded FC"
                )));
            }
            let start = (fc / 2)
                .checked_add(skip_cp)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let end = start
                .checked_add(take_cp as usize)
                .filter(|end| *end <= word_document.len())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            out.extend(
                word_document[start..end]
                    .iter()
                    .copied()
                    .map(decode_ansi_byte),
            );
        } else {
            let start = fc
                .checked_add(
                    skip_cp
                        .checked_mul(2)
                        .ok_or(DocError::PieceOutOfBounds { piece: index })?,
                )
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let byte_len = take_cp
                .checked_mul(2)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let end = start
                .checked_add(byte_len)
                .filter(|end| *end <= word_document.len())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let units = word_document[start..end]
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
            for decoded in decode_utf16(units) {
                out.push(decoded.map_err(|_| DocError::InvalidUtf16 { piece: index })?);
            }
        }
    }
    if out.len() != main_cp as usize {
        return Err(DocError::MalformedClx(format!(
            "piece table reconstructed {} main-story CPs, expected {main_cp}",
            out.len()
        )));
    }
    Ok(out)
}

fn reconstruct_text_range(
    word_document: &[u8],
    pieces: &[Piece],
    range_start: u32,
    range_end: u32,
) -> Result<Vec<char>, DocError> {
    let mut out = Vec::with_capacity(range_end.saturating_sub(range_start) as usize);
    for (index, piece) in pieces.iter().enumerate() {
        let cp_start = piece.cp_start.max(range_start);
        let cp_end = piece.cp_end.min(range_end);
        if cp_start >= cp_end {
            continue;
        }
        let skip_cp = cp_start - piece.cp_start;
        let take_cp = cp_end - cp_start;
        let compressed = piece.raw_fc & 0x4000_0000 != 0;
        let fc = piece.raw_fc & 0x3FFF_FFFF;
        if compressed {
            let start = (fc / 2)
                .checked_add(skip_cp)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let end = start
                .checked_add(take_cp as usize)
                .filter(|end| *end <= word_document.len())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            out.extend(
                word_document[start..end]
                    .iter()
                    .copied()
                    .map(decode_ansi_byte),
            );
        } else {
            let start = fc
                .checked_add(skip_cp.saturating_mul(2))
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let end = start
                .checked_add(take_cp.saturating_mul(2) as usize)
                .filter(|end| *end <= word_document.len())
                .ok_or(DocError::PieceOutOfBounds { piece: index })?;
            let units = word_document[start..end]
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
            for decoded in decode_utf16(units) {
                out.push(decoded.map_err(|_| DocError::InvalidUtf16 { piece: index })?);
            }
        }
    }
    Ok(out)
}

#[derive(Default)]
struct TextboxPlacements {
    by_anchor: BTreeMap<u32, Vec<String>>,
    unplaced: usize,
}

fn parse_textboxes(
    word_document: &[u8],
    table_stream: &[u8],
    fib: &Fib,
    pieces: &[Piece],
) -> Result<TextboxPlacements, String> {
    if fib.stories.textboxes == 0 || fib.lcb_plcf_txbx_txt == 0 {
        return Ok(TextboxPlacements::default());
    }
    let plc = checked_range(
        table_stream,
        fib.fc_plcf_txbx_txt,
        fib.lcb_plcf_txbx_txt,
        "PlcftxbxTxt",
    )?;
    if plc.len() < 30 || (plc.len() - 4) % 26 != 0 {
        return Err(format!("invalid PlcftxbxTxt length {}", plc.len()));
    }
    let count = (plc.len() - 4) / 26;
    let cp_bytes = (count + 1) * 4;
    let textbox_story_start = fib
        .stories
        .main
        .saturating_add(fib.stories.footnotes)
        .saturating_add(fib.stories.headers)
        .saturating_add(fib.stories.comments)
        .saturating_add(fib.stories.endnotes);
    let mut textboxes = Vec::new();
    for index in 0..count.saturating_sub(1) {
        let record = cp_bytes + index * 22;
        let reusable = read_u16_at(plc, record + 8, "FTXBXS.fReusable")? & 1 != 0;
        if reusable {
            continue;
        }
        let lid = read_u32_at(plc, record + 14, "FTXBXS.lid")?;
        let start = read_u32_at(plc, index * 4, "PlcftxbxTxt.aCP")?;
        let end = read_u32_at(plc, (index + 1) * 4, "PlcftxbxTxt.aCP")?;
        if start >= end || end > fib.stories.textboxes {
            continue;
        }
        let chars = reconstruct_text_range(
            word_document,
            pieces,
            textbox_story_start.saturating_add(start),
            textbox_story_start.saturating_add(end),
        )
        .map_err(|error| error.to_string())?;
        let text: String = chars
            .into_iter()
            .filter(|character| !matches!(character, '\u{0007}'))
            .map(|character| if character == '\r' { ' ' } else { character })
            .collect::<String>()
            .trim()
            .to_owned();
        if !text.is_empty() {
            textboxes.push((lid, text));
        }
    }

    let spa = checked_range(
        table_stream,
        fib.fc_plc_spa_mom,
        fib.lcb_plc_spa_mom,
        "PlcfSpaMom",
    )?;
    if spa.len() < 30 || (spa.len() - 4) % 30 != 0 {
        return Err(format!("invalid PlcfSpaMom length {}", spa.len()));
    }
    let anchor_count = (spa.len() - 4) / 30;
    let spa_records = (anchor_count + 1) * 4;
    let mut anchors = Vec::with_capacity(anchor_count);
    for index in 0..anchor_count {
        let cp = read_u32_at(spa, index * 4, "PlcfSpaMom.aCP")?;
        let lid = read_u32_at(spa, spa_records + index * 26, "SPA.lid")?;
        anchors.push((cp, lid));
    }

    let mut placements = TextboxPlacements::default();
    let mut by_lid: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (lid, text) in &textboxes {
        by_lid.entry(*lid).or_default().push(text.clone());
    }
    for (cp, lid) in &anchors {
        if let Some(text) = by_lid.remove(lid) {
            placements.by_anchor.entry(*cp).or_default().extend(text);
        }
    }

    let placed = placements.by_anchor.values().map(Vec::len).sum::<usize>();
    if placed == 0 && textboxes.len() == anchors.len() {
        for ((_, text), (cp, _)) in textboxes.into_iter().zip(anchors) {
            placements.by_anchor.entry(cp).or_default().push(text);
        }
    } else {
        placements.unplaced = textboxes.len().saturating_sub(placed);
    }
    Ok(placements)
}

struct FormattingContext<'a> {
    word_document: &'a [u8],
    table_stream: &'a [u8],
    data_stream: &'a [u8],
    fib: &'a Fib,
    pieces: &'a [Piece],
    text: &'a [char],
}

fn apply_paragraph_formatting(
    context: FormattingContext<'_>,
    paragraph_cps: &[u32],
    document: &mut Document,
) -> Result<usize, String> {
    let plc = checked_range(
        context.table_stream,
        context.fib.fc_plcf_bte_papx,
        context.fib.lcb_plcf_bte_papx,
        "PlcBtePapx",
    )?;
    if plc.len() < 12 || (plc.len() - 4) % 8 != 0 {
        return Err(format!("invalid PlcBtePapx length {}", plc.len()));
    }
    let page_count = (plc.len() - 4) / 8;
    let fc_count = page_count + 1;
    let page_numbers_offset = fc_count * 4;
    let mut formatted = 0_usize;

    for (paragraph, &cp) in document.paragraphs.iter_mut().zip(paragraph_cps) {
        let Some(fc) = cp_to_fc(context.pieces, cp) else {
            continue;
        };
        let bte_index = find_interval_u32(plc, 0, fc_count, fc, "PlcBtePapx.aFC")?;
        let Some(bte_index) = bte_index.filter(|index| *index < page_count) else {
            continue;
        };
        let pn_raw = read_u32_at(plc, page_numbers_offset + bte_index * 4, "PnFkpPapx")?;
        let page_offset = usize::try_from(pn_raw & 0x003F_FFFF)
            .ok()
            .and_then(|pn| pn.checked_mul(512))
            .ok_or_else(|| "PapxFkp offset overflowed".to_owned())?;
        let page = context
            .word_document
            .get(page_offset..page_offset + 512)
            .ok_or_else(|| format!("PapxFkp at {page_offset} is outside WordDocument"))?;
        let cpara = usize::from(page[511]);
        if !(1..=0x1D).contains(&cpara) {
            return Err(format!("PapxFkp has invalid cpara {cpara}"));
        }
        let run_index = find_interval_u32(page, 0, cpara + 1, fc, "PapxFkp.rgfc")?;
        let Some(run_index) = run_index.filter(|index| *index < cpara) else {
            continue;
        };
        let bx_offset = (cpara + 1) * 4 + run_index * 13;
        let b_offset = usize::from(page[bx_offset]);
        if b_offset == 0 {
            formatted += 1;
            continue;
        }
        let grpprl = papx_grpprl(page, b_offset * 2)?;
        paragraph.properties = decode_paragraph_properties(grpprl, context.data_stream)?;
        paragraph.properties.table.cell_end = context
            .text
            .get(usize::try_from(cp).unwrap_or(usize::MAX))
            .is_some_and(|character| *character == '\u{0007}');
        formatted += 1;
    }
    Ok(formatted)
}

#[derive(Debug)]
struct ListTemplate {
    lsid: i32,
    levels: Vec<NumberingLevel>,
}

fn parse_numbering(table_stream: &[u8], fib: &Fib) -> Result<Vec<NumberingDefinition>, String> {
    if fib.lcb_plf_lst == 0 || fib.lcb_plf_lfo == 0 {
        return Ok(Vec::new());
    }
    let plf_lst = checked_range(table_stream, fib.fc_plf_lst, fib.lcb_plf_lst, "PlfLst")?;
    let list_count = usize::from(read_u16_at(plf_lst, 0, "PlfLst.cLst")?);
    let lstf_bytes = list_count
        .checked_mul(28)
        .and_then(|size| size.checked_add(2))
        .ok_or_else(|| "PlfLst size overflowed".to_owned())?;
    if lstf_bytes > plf_lst.len() {
        return Err("PlfLst.rgLstf is truncated".to_owned());
    }

    let mut level_cursor = usize::try_from(fib.fc_plf_lst)
        .ok()
        .and_then(|offset| offset.checked_add(usize::try_from(fib.lcb_plf_lst).ok()?))
        .ok_or_else(|| "appended LVL offset overflowed".to_owned())?;
    let mut templates = Vec::with_capacity(list_count);
    for index in 0..list_count {
        let lstf_offset = 2 + index * 28;
        let lsid = read_i32_at(plf_lst, lstf_offset, "LSTF.lsid")?;
        let flags = plf_lst[lstf_offset + 26];
        let level_count = if flags & 0x01 != 0 { 1 } else { 9 };
        let mut levels = Vec::with_capacity(level_count);
        for level in 0..level_count {
            let (parsed, next) = parse_numbering_level(table_stream, level_cursor, level as u8)?;
            levels.push(parsed);
            level_cursor = next;
        }
        templates.push(ListTemplate { lsid, levels });
    }

    let plf_lfo = checked_range(table_stream, fib.fc_plf_lfo, fib.lcb_plf_lfo, "PlfLfo")?;
    let lfo_count = usize::try_from(read_u32_at(plf_lfo, 0, "PlfLfo.lfoMac")?)
        .map_err(|_| "PlfLfo.lfoMac overflowed".to_owned())?;
    let lfo_bytes = lfo_count
        .checked_mul(16)
        .and_then(|size| size.checked_add(4))
        .ok_or_else(|| "PlfLfo size overflowed".to_owned())?;
    if lfo_bytes > plf_lfo.len() {
        return Err("PlfLfo.rgLfo is truncated".to_owned());
    }
    let mut lfo_metadata = Vec::with_capacity(lfo_count);
    for index in 0..lfo_count {
        let offset = 4 + index * 16;
        let lsid = read_i32_at(plf_lfo, offset, "LFO.lsid")?;
        let override_count = usize::from(plf_lfo[offset + 12]);
        lfo_metadata.push((lsid, override_count));
    }
    let mut data_cursor = lfo_bytes;
    let mut definitions = Vec::with_capacity(lfo_count);
    for (index, (lsid, override_count)) in lfo_metadata.into_iter().enumerate() {
        let _cp = read_u32_at(plf_lfo, data_cursor, "LFOData.cp")?;
        data_cursor += 4;
        let mut overrides = Vec::with_capacity(override_count);
        for _ in 0..override_count {
            let start_at = read_i32_at(plf_lfo, data_cursor, "LFOLVL.iStartAt")?;
            let flags = read_u32_at(plf_lfo, data_cursor + 4, "LFOLVL.flags")?;
            let level = (flags & 0x0F) as u8;
            let has_start = flags & 0x10 != 0;
            let has_formatting = flags & 0x20 != 0;
            data_cursor += 8;
            let formatting = if has_formatting {
                let (formatting, next) = parse_numbering_level(plf_lfo, data_cursor, level)?;
                data_cursor = next;
                Some(formatting)
            } else {
                None
            };
            overrides.push(NumberingOverride {
                level,
                start: if has_start && !has_formatting {
                    u32::try_from(start_at).ok()
                } else {
                    None
                },
                formatting,
            });
        }
        if let Some(template) = templates.iter().find(|template| template.lsid == lsid) {
            definitions.push(NumberingDefinition {
                instance: (index + 1) as u32,
                levels: template.levels.clone(),
                overrides,
            });
        }
    }
    Ok(definitions)
}

fn parse_numbering_level(
    table_stream: &[u8],
    offset: usize,
    level: u8,
) -> Result<(NumberingLevel, usize), String> {
    let lvlf = table_stream
        .get(offset..offset + 28)
        .ok_or_else(|| "LVLF is truncated".to_owned())?;
    let start = read_i32_at(lvlf, 0, "LVLF.iStartAt")?;
    let nfc = lvlf[4];
    let alignment = match lvlf[5] & 0x03 {
        1 => Alignment::Center,
        2 => Alignment::Right,
        _ => Alignment::Left,
    };
    let placeholder_positions = &lvlf[6..15];
    let suffix = match lvlf[15] {
        1 => NumberingSuffix::Space,
        2 => NumberingSuffix::Nothing,
        _ => NumberingSuffix::Tab,
    };
    let chpx_len = usize::from(lvlf[24]);
    let papx_len = usize::from(lvlf[25]);
    let papx_start = offset + 28;
    let chpx_start = papx_start
        .checked_add(papx_len)
        .ok_or_else(|| "LVL grpprl offset overflowed".to_owned())?;
    let xst_start = chpx_start
        .checked_add(chpx_len)
        .ok_or_else(|| "LVL xst offset overflowed".to_owned())?;
    let papx = table_stream
        .get(papx_start..chpx_start)
        .ok_or_else(|| "LVL.grpprlPapx is truncated".to_owned())?;
    let cch = usize::from(read_u16_at(table_stream, xst_start, "LVL.xst.cch")?);
    let text_start = xst_start + 2;
    let text_end = text_start
        .checked_add(cch * 2)
        .ok_or_else(|| "LVL.xst size overflowed".to_owned())?;
    let text_bytes = table_stream
        .get(text_start..text_end)
        .ok_or_else(|| "LVL.xst is truncated".to_owned())?;
    let units: Vec<u16> = text_bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    let text = if nfc == 0x17 {
        bullet_text(&units)
    } else {
        let mut text = String::new();
        for (index, unit) in units.into_iter().enumerate() {
            let position = u8::try_from(index + 1).unwrap_or(u8::MAX);
            if placeholder_positions.contains(&position) && unit <= 8 {
                text.push('%');
                text.push_str(&(u32::from(unit) + 1).to_string());
            } else {
                text.extend(decode_utf16([unit]).map(|value| value.unwrap_or('\u{FFFD}')));
            }
        }
        text
    };
    let mut paragraph = decode_paragraph_properties(papx, &[])?;
    paragraph.numbering = None;
    Ok((
        NumberingLevel {
            level,
            start: u32::try_from(start).unwrap_or(1),
            format: numbering_format(nfc).to_owned(),
            text,
            alignment,
            suffix,
            paragraph,
        },
        text_end,
    ))
}

fn bullet_text(units: &[u16]) -> String {
    let Some(&raw) = units.first() else {
        return "•".to_owned();
    };
    decode_utf16([raw])
        .map(|value| value.unwrap_or('•'))
        .collect()
}

fn numbering_format(nfc: u8) -> &'static str {
    match nfc {
        0x00 | 0x28 => "decimal",
        0x01 => "upperRoman",
        0x02 => "lowerRoman",
        0x03 => "upperLetter",
        0x04 => "lowerLetter",
        0x05 => "ordinal",
        0x06 => "cardinalText",
        0x07 => "ordinalText",
        0x08 => "hex",
        0x09 => "chicago",
        0x0A => "ideographDigital",
        0x0B => "japaneseCounting",
        0x0C => "Aiueo",
        0x0D => "Iroha",
        0x0E => "decimalFullWidth",
        0x0F => "decimalHalfWidth",
        0x10 => "japaneseLegal",
        0x11 => "japaneseDigitalTenThousand",
        0x12 => "decimalEnclosedCircle",
        0x13 => "decimalFullWidth2",
        0x14 => "aiueoFullWidth",
        0x15 => "irohaFullWidth",
        0x16 => "decimalZero",
        0x17 => "bullet",
        0x18 => "ganada",
        0x19 => "chosung",
        0x1A => "decimalEnclosedFullstop",
        0x1B => "decimalEnclosedParen",
        0x1C => "decimalEnclosedCircleChinese",
        0x1D => "ideographEnclosedCircle",
        0x1E => "ideographTraditional",
        0x1F => "ideographZodiac",
        0x20 => "ideographZodiacTraditional",
        0x21 => "taiwaneseCounting",
        0x22 => "ideographLegalTraditional",
        0x23 => "taiwaneseCountingThousand",
        0x24 => "taiwaneseDigital",
        0x25 => "chineseCounting",
        0x26 => "chineseLegalSimplified",
        0x27 => "chineseCountingThousand",
        0x29 => "koreanDigital",
        0x2A => "koreanCounting",
        0x2B => "koreanLegal",
        0x2C => "koreanDigital2",
        0x2D => "hebrew1",
        0x2E => "arabicAlpha",
        0x2F => "hebrew2",
        0x30 => "arabicAbjad",
        0x31 => "hindiVowels",
        0x32 => "hindiConsonants",
        0x33 => "hindiNumbers",
        0x34 => "hindiCounting",
        0x35 => "thaiLetters",
        0x36 => "thaiNumbers",
        0x37 => "thaiCounting",
        0x38 => "vietnameseCounting",
        0x39 => "numberInDash",
        0x3A => "russianLower",
        0x3B => "russianUpper",
        0xFF => "none",
        _ => "decimal",
    }
}

fn parse_section_properties(
    word_document: &[u8],
    table_stream: &[u8],
    fib: &Fib,
) -> Result<(SectionProperties, usize), String> {
    let plc = checked_range(table_stream, fib.fc_plcf_sed, fib.lcb_plcf_sed, "PlcfSed")?;
    if plc.len() < 20 || (plc.len() - 4) % 16 != 0 {
        return Err(format!("invalid PlcfSed length {}", plc.len()));
    }
    let section_count = (plc.len() - 4) / 16;
    let cp_bytes = (section_count + 1) * 4;
    let sed_offset = cp_bytes + (section_count - 1) * 12;
    let fc_sepx = read_i32_at(plc, sed_offset + 2, "Sed.fcSepx")?;
    if fc_sepx < 0 {
        return Ok((SectionProperties::default(), section_count));
    }
    let sepx_offset = usize::try_from(fc_sepx).map_err(|_| "fcSepx overflowed".to_owned())?;
    let cb = usize::from(read_u16_at(word_document, sepx_offset, "Sepx.cb")?);
    let start = sepx_offset + 2;
    let grpprl = word_document
        .get(start..start + cb)
        .ok_or_else(|| "Sepx.grpprl is outside WordDocument".to_owned())?;
    Ok((decode_section_properties(grpprl)?, section_count))
}

fn decode_paragraph_properties(
    grpprl: &[u8],
    data_stream: &[u8],
) -> Result<ParagraphProperties, String> {
    decode_paragraph_properties_inner(grpprl, data_stream, 0)
}

fn decode_paragraph_properties_inner(
    grpprl: &[u8],
    data_stream: &[u8],
    depth: usize,
) -> Result<ParagraphProperties, String> {
    if depth > 8 {
        return Err("paragraph property indirection is too deep".to_owned());
    }
    let mut indirect = None;
    for_each_prl(grpprl, |sprm, operand| {
        if matches!(sprm, 0x6646 | 0x646B) && operand.len() >= 4 {
            indirect = Some(u32_from(operand));
        }
        Ok(())
    })?;
    if let Some(offset) = indirect {
        let offset = usize::try_from(offset).map_err(|_| "PrcData offset overflowed".to_owned())?;
        let length = usize::from(read_u16_at(data_stream, offset, "PrcData.cbGrpprl")?);
        let referenced = data_stream
            .get(offset + 2..offset + 2 + length)
            .ok_or_else(|| "PrcData.grpprl is outside Data stream".to_owned())?;
        return decode_paragraph_properties_inner(referenced, data_stream, depth + 1);
    }

    let mut properties = ParagraphProperties::default();
    let mut physical_left = None;
    let mut physical_right = None;
    let mut logical_start = None;
    let mut logical_end = None;
    let mut nest = 0_i16;
    let mut list_level = 0_u8;
    let mut list_instance = None;

    for_each_prl(grpprl, |sprm, operand| {
        match sprm {
            0x260A if !operand.is_empty() => list_level = operand[0],
            0x460B if operand.len() >= 2 => {
                let value = i16_from(operand);
                if value != 0 && value != -2047 {
                    list_instance = Some(u32::from(value.unsigned_abs()));
                } else {
                    list_instance = None;
                }
            }
            0x2403 | 0x2461 if !operand.is_empty() => {
                properties.alignment = alignment_from_u8(operand[0]);
            }
            0x2441 if !operand.is_empty() => properties.bidi = Some(operand[0] != 0),
            0x2416 if !operand.is_empty() => properties.table.in_table = operand[0] != 0,
            0x2417 if !operand.is_empty() => properties.table.row_end = operand[0] != 0,
            0x6649 if operand.len() >= 4 => {
                properties.table.depth = u32_from(operand).min(u32::from(u8::MAX)) as u8;
                properties.table.in_table = properties.table.depth != 0;
            }
            0x664A if operand.len() >= 4 => {
                let delta = i32_from(operand);
                properties.table.depth = i32::from(properties.table.depth)
                    .saturating_add(delta)
                    .clamp(0, i32::from(u8::MAX)) as u8;
                properties.table.in_table = properties.table.depth != 0;
            }
            0x560B | 0x5664 if operand.len() >= 2 => {
                properties.table.rtl = u16_from(operand) != 0;
                properties.table.reverse_cells = properties.table.rtl && depth == 0;
            }
            0x840E if operand.len() >= 2 => physical_right = Some(i16_from(operand)),
            0x840F if operand.len() >= 2 => physical_left = Some(i16_from(operand)),
            0x845D if operand.len() >= 2 => logical_end = Some(i16_from(operand)),
            0x845E if operand.len() >= 2 => logical_start = Some(i16_from(operand)),
            0x8411 | 0x8460 if operand.len() >= 2 => {
                properties.indent_first_line = Some(i16_from(operand));
            }
            0x4610 | 0x465F if operand.len() >= 2 => nest = i16_from(operand),
            0xA413 if operand.len() >= 2 => properties.space_before = Some(u16_from(operand)),
            0xA414 if operand.len() >= 2 => properties.space_after = Some(u16_from(operand)),
            0x6424 if operand.len() >= 4 => properties.borders.top = decode_border80(operand),
            0x6425 if operand.len() >= 4 => properties.borders.start = decode_border80(operand),
            0x6426 if operand.len() >= 4 => properties.borders.bottom = decode_border80(operand),
            0x6427 if operand.len() >= 4 => properties.borders.end = decode_border80(operand),
            0x6428 if operand.len() >= 4 => properties.borders.between = decode_border80(operand),
            0xC64D if operand.len() >= 11 && operand[0] == 10 => {
                properties.shading = decode_shading(&operand[1..11]);
            }
            0xC64E if operand.len() >= 9 && operand[0] == 8 => {
                properties.borders.top = decode_border(&operand[1..9]);
            }
            0xC64F if operand.len() >= 9 && operand[0] == 8 => {
                properties.borders.start = decode_border(&operand[1..9]);
            }
            0xC650 if operand.len() >= 9 && operand[0] == 8 => {
                properties.borders.bottom = decode_border(&operand[1..9]);
            }
            0xC651 if operand.len() >= 9 && operand[0] == 8 => {
                properties.borders.end = decode_border(&operand[1..9]);
            }
            0xC652 if operand.len() >= 9 && operand[0] == 8 => {
                properties.borders.between = decode_border(&operand[1..9]);
            }
            _ => {}
        }
        Ok(())
    })?;

    let bidi = properties.bidi.unwrap_or(false);
    properties.indent_start = logical_start.or(if bidi { physical_right } else { physical_left });
    properties.indent_end = logical_end.or(if bidi { physical_left } else { physical_right });
    if nest != 0 {
        properties.indent_start = Some(properties.indent_start.unwrap_or(0).saturating_add(nest));
    }
    if list_level <= 8 {
        properties.numbering = list_instance.map(|instance| NumberingRef {
            instance,
            level: list_level,
        });
    }
    if properties.table.in_table && properties.table.depth == 0 {
        properties.table.depth = 1;
    }
    Ok(properties)
}

fn decode_border(bytes: &[u8]) -> Option<Border> {
    let style = border_style(bytes[5]);
    if style == "none" {
        return None;
    }
    let flags = u16_from(&bytes[6..8]);
    Some(Border {
        style: style.to_owned(),
        size: bytes[4].max(2),
        space: (flags & 0x1F) as u8,
        color: colorref(&bytes[0..4]),
        shadow: flags & 0x20 != 0,
        frame: flags & 0x40 != 0,
    })
}

fn decode_border80(bytes: &[u8]) -> Option<Border> {
    let style = border_style(bytes[1]);
    if style == "none" || bytes == [0xFF; 4] {
        return None;
    }
    Some(Border {
        style: style.to_owned(),
        size: bytes[0].max(2),
        space: bytes[3] & 0x1F,
        color: indexed_color(bytes[2]),
        shadow: bytes[3] & 0x20 != 0,
        frame: bytes[3] & 0x40 != 0,
    })
}

fn decode_shading(bytes: &[u8]) -> Option<Shading> {
    let pattern = u16_from(&bytes[8..10]);
    if pattern == 0 && colorref(&bytes[4..8]).is_none() {
        return None;
    }
    Some(Shading {
        pattern: shading_pattern(pattern).to_owned(),
        foreground: colorref(&bytes[0..4]),
        background: colorref(&bytes[4..8]),
    })
}

fn border_style(value: u8) -> &'static str {
    match value {
        0x00 => "none",
        0x01 | 0x05 => "single",
        0x03 => "double",
        0x06 => "dotted",
        0x07 => "dashed",
        0x08 => "dotDash",
        0x09 => "dotDotDash",
        0x0A => "triple",
        0x0B => "thinThickSmallGap",
        0x0C => "thickThinSmallGap",
        0x0D => "thinThickThinSmallGap",
        0x0E => "thinThickMediumGap",
        0x0F => "thickThinMediumGap",
        0x10 => "thinThickThinMediumGap",
        0x11 => "thinThickLargeGap",
        0x12 => "thickThinLargeGap",
        0x13 => "thinThickThinLargeGap",
        0x14 => "wave",
        0x15 => "doubleWave",
        0x16 => "dashSmallGap",
        0x17 => "dashDotStroked",
        0x18 => "threeDEmboss",
        0x19 => "threeDEngrave",
        0x1A => "outset",
        0x1B => "inset",
        _ => "single",
    }
}

fn shading_pattern(value: u16) -> &'static str {
    match value {
        0 => "clear",
        1 => "solid",
        2 => "pct5",
        3 => "pct10",
        4 => "pct20",
        5 => "pct25",
        6 => "pct30",
        7 => "pct40",
        8 => "pct50",
        9 => "pct60",
        10 => "pct70",
        11 => "pct75",
        12 => "pct80",
        13 => "pct90",
        14 => "horzStripe",
        15 => "vertStripe",
        16 => "reverseDiagStripe",
        17 => "diagStripe",
        18 => "horzCross",
        19 => "diagCross",
        20 => "thinHorzStripe",
        21 => "thinVertStripe",
        22 => "thinReverseDiagStripe",
        23 => "thinDiagStripe",
        24 => "thinHorzCross",
        25 => "thinDiagCross",
        0x25 => "pct12",
        0x26 => "pct15",
        _ => "clear",
    }
}

fn colorref(bytes: &[u8]) -> Option<String> {
    if bytes[3] != 0 || bytes[..3] == [0xFF; 3] {
        None
    } else {
        Some(format!("{:02X}{:02X}{:02X}", bytes[0], bytes[1], bytes[2]))
    }
}

fn indexed_color(value: u8) -> Option<String> {
    let color = match value {
        1 => "000000",
        2 => "0000FF",
        3 => "00FFFF",
        4 => "00FF00",
        5 => "FF00FF",
        6 => "FF0000",
        7 => "FFFF00",
        8 => "FFFFFF",
        9 => "000080",
        10 => "008080",
        11 => "008000",
        12 => "800080",
        13 => "800000",
        14 => "808000",
        15 => "808080",
        16 => "C0C0C0",
        _ => return None,
    };
    Some(color.to_owned())
}

fn decode_section_properties(grpprl: &[u8]) -> Result<SectionProperties, String> {
    let mut properties = SectionProperties::default();
    for_each_prl(grpprl, |sprm, operand| {
        match sprm {
            0x301D if !operand.is_empty() => properties.landscape = Some(operand[0] == 2),
            0xB01F if operand.len() >= 2 => properties.page_width = Some(u16_from(operand)),
            0xB020 if operand.len() >= 2 => properties.page_height = Some(u16_from(operand)),
            0xB021 if operand.len() >= 2 => properties.margin_left = Some(u16_from(operand)),
            0xB022 if operand.len() >= 2 => properties.margin_right = Some(u16_from(operand)),
            0x9023 if operand.len() >= 2 => properties.margin_top = Some(i16_from(operand)),
            0x9024 if operand.len() >= 2 => properties.margin_bottom = Some(i16_from(operand)),
            0x3228 if !operand.is_empty() => properties.bidi = Some(operand[0] != 0),
            0x322A if !operand.is_empty() => properties.rtl_gutter = Some(operand[0] != 0),
            _ => {}
        }
        Ok(())
    })?;
    Ok(properties)
}

fn for_each_prl(
    grpprl: &[u8],
    mut visit: impl FnMut(u16, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut cursor = 0_usize;
    while cursor < grpprl.len() {
        let sprm = read_u16_at(grpprl, cursor, "Sprm")?;
        cursor += 2;
        let spra = sprm >> 13;
        let operand_len = match spra {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            6 => {
                if sprm == 0xD608 {
                    usize::from(read_u16_at(grpprl, cursor, "TDefTableOperand.cb")?) + 1
                } else {
                    let size = *grpprl
                        .get(cursor)
                        .ok_or_else(|| format!("variable-length sprm 0x{sprm:04X} is truncated"))?;
                    usize::from(size) + 1
                }
            }
            _ => unreachable!(),
        };
        let operand = grpprl
            .get(cursor..cursor + operand_len)
            .ok_or_else(|| format!("operand for sprm 0x{sprm:04X} is truncated"))?;
        visit(sprm, operand)?;
        cursor += operand_len;
    }
    Ok(())
}

fn papx_grpprl(page: &[u8], offset: usize) -> Result<&[u8], String> {
    let cb = usize::from(
        *page
            .get(offset)
            .ok_or_else(|| "PapxInFkp.cb is outside page".to_owned())?,
    );
    let (start, length) = if cb == 0 {
        let extended = usize::from(
            *page
                .get(offset + 1)
                .ok_or_else(|| "PapxInFkp extended cb is missing".to_owned())?,
        );
        (offset + 2, extended * 2)
    } else {
        (offset + 1, cb * 2 - 1)
    };
    let grpprl_and_istd = page
        .get(start..start + length)
        .ok_or_else(|| "PapxInFkp extends outside page".to_owned())?;
    grpprl_and_istd
        .get(2..)
        .ok_or_else(|| "GrpprlAndIstd is missing istd".to_owned())
}

fn alignment_from_u8(value: u8) -> Option<Alignment> {
    match value {
        0 => Some(Alignment::Left),
        1 => Some(Alignment::Center),
        2 => Some(Alignment::Right),
        3 => Some(Alignment::Both),
        4 => Some(Alignment::Distribute),
        5 => Some(Alignment::MediumKashida),
        7 => Some(Alignment::HighKashida),
        8 => Some(Alignment::LowKashida),
        9 => Some(Alignment::ThaiDistribute),
        _ => None,
    }
}

fn cp_to_fc(pieces: &[Piece], cp: u32) -> Option<u32> {
    let piece = pieces
        .iter()
        .find(|piece| cp >= piece.cp_start && cp < piece.cp_end)?;
    let delta = cp.checked_sub(piece.cp_start)?;
    let encoded_fc = piece.raw_fc & 0x3FFF_FFFF;
    if piece.raw_fc & 0x4000_0000 != 0 {
        (encoded_fc / 2).checked_add(delta)
    } else {
        encoded_fc.checked_add(delta.checked_mul(2)?)
    }
}

fn find_interval_u32(
    data: &[u8],
    offset: usize,
    count: usize,
    value: u32,
    field: &str,
) -> Result<Option<usize>, String> {
    let mut previous = None;
    let mut result = None;
    for index in 0..count {
        let current = read_u32_at(data, offset + index * 4, field)?;
        if previous.is_some_and(|prior| current <= prior) {
            return Err(format!("{field} is not strictly increasing"));
        }
        if current <= value {
            result = Some(index);
        }
        previous = Some(current);
    }
    Ok(result)
}

fn checked_range<'a>(
    data: &'a [u8],
    offset: u32,
    length: u32,
    field: &str,
) -> Result<&'a [u8], String> {
    let start = usize::try_from(offset).map_err(|_| format!("{field} offset overflowed"))?;
    let length = usize::try_from(length).map_err(|_| format!("{field} length overflowed"))?;
    data.get(start..start + length)
        .ok_or_else(|| format!("{field} is outside its stream"))
}

fn text_to_ir(
    chars: &[char],
    symbols: &[Option<u16>],
    textboxes: &TextboxPlacements,
    features_seen: &mut BTreeSet<String>,
    unsupported: &mut BTreeSet<String>,
    warnings: &mut Vec<String>,
) -> (Document, Vec<u32>) {
    let mut builder = IrBuilder::default();
    let mut field_stack: Vec<bool> = Vec::new();
    let mut pending_textboxes: Vec<&str> = Vec::new();

    for (cp, &ch) in chars.iter().enumerate() {
        if let Some(character) = symbols.get(cp).copied().flatten() {
            builder.push_inline(Inline::Symbol {
                font: "Symbol".to_owned(),
                character,
            });
            continue;
        }
        match ch {
            '\u{0013}' => {
                features_seen.insert("fields".to_owned());
                unsupported.insert("fields".to_owned());
                field_stack.push(false);
            }
            '\u{0014}' if !field_stack.is_empty() => {
                if let Some(frame) = field_stack.last_mut() {
                    *frame = true;
                }
            }
            '\u{0015}' if !field_stack.is_empty() => {
                field_stack.pop();
            }
            _ if field_stack.iter().any(|in_result| !*in_result) => {}
            '\r' => {
                push_pending_textboxes(&mut builder, &mut pending_textboxes);
                builder.end_paragraph(cp as u32);
            }
            '\t' => {
                features_seen.insert("tabs".to_owned());
                builder.push_inline(Inline::Tab);
            }
            '\n' | '\u{000B}' => builder.push_inline(Inline::Break(BreakKind::Line)),
            '\u{000C}' => {
                features_seen.insert("page-breaks".to_owned());
                builder.push_inline(Inline::Break(BreakKind::Page));
            }
            '\u{000E}' => {
                features_seen.insert("column-breaks".to_owned());
                builder.push_inline(Inline::Break(BreakKind::Column));
            }
            '\u{0007}' => {
                features_seen.insert("tables".to_owned());
                unsupported.insert("tables".to_owned());
                push_pending_textboxes(&mut builder, &mut pending_textboxes);
                builder.end_paragraph(cp as u32);
            }
            '\u{0001}' => {
                features_seen.insert("inline-objects".to_owned());
                unsupported.insert("inline-objects".to_owned());
                builder.push_char('\u{FFFC}');
            }
            '\u{0008}' => {
                features_seen.insert("inline-objects".to_owned());
                unsupported.insert("inline-objects".to_owned());
                if let Some(anchored) = textboxes.by_anchor.get(&(cp as u32)) {
                    pending_textboxes.extend(anchored.iter().map(String::as_str));
                }
                builder.push_char('\u{FFFC}');
            }
            '\u{0002}' => {
                features_seen.insert("note-references".to_owned());
                unsupported.insert("note-references".to_owned());
                builder.push_note_reference();
            }
            '\u{001E}' => builder.push_char('\u{2011}'),
            '\u{001F}' => builder.push_char('\u{00AD}'),
            control if control <= '\u{001F}' => {
                let code = format!("control:0x{:02X}", control as u32);
                unsupported.insert(code.clone());
                warnings.push(format!("preserved unsupported {code} as U+FFFC"));
                builder.push_char('\u{FFFC}');
            }
            visible => builder.push_char(visible),
        }
    }
    if !field_stack.is_empty() {
        unsupported.insert("fields:unbalanced".to_owned());
        warnings.push("field delimiters are unbalanced".to_owned());
    }
    push_pending_textboxes(&mut builder, &mut pending_textboxes);
    builder.finish(chars.len() as u32)
}

fn push_pending_textboxes(builder: &mut IrBuilder, pending: &mut Vec<&str>) {
    if pending.is_empty() {
        return;
    }
    builder.push_char(' ');
    for textbox in pending.drain(..) {
        for character in textbox.chars() {
            match character {
                '\t' => builder.push_inline(Inline::Tab),
                '\n' | '\u{000B}' => builder.push_inline(Inline::Break(BreakKind::Line)),
                control if control <= '\u{001F}' => builder.push_char('\u{FFFC}'),
                visible => builder.push_char(visible),
            }
        }
    }
    builder.push_char(' ');
}

#[derive(Default)]
struct IrBuilder {
    document: Document,
    current: Paragraph,
    text: String,
    ended_paragraph: bool,
    paragraph_cps: Vec<u32>,
    note_reference: u32,
}

impl IrBuilder {
    fn push_char(&mut self, ch: char) {
        self.text.push(ch);
        self.ended_paragraph = false;
    }

    fn push_note_reference(&mut self) {
        self.note_reference = self.note_reference.saturating_add(1);
        self.text.push('\u{FFFC}');
        self.ended_paragraph = false;
    }

    fn push_inline(&mut self, inline: Inline) {
        self.flush_text();
        self.current.children.push(inline);
        self.ended_paragraph = false;
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.current
                .children
                .push(Inline::Text(std::mem::take(&mut self.text)));
        }
    }

    fn end_paragraph(&mut self, cp: u32) {
        self.flush_text();
        self.document
            .paragraphs
            .push(std::mem::take(&mut self.current));
        self.paragraph_cps.push(cp);
        self.ended_paragraph = true;
    }

    fn finish(mut self, cp_lim: u32) -> (Document, Vec<u32>) {
        self.flush_text();
        if !self.ended_paragraph || self.document.paragraphs.is_empty() {
            self.document.paragraphs.push(self.current);
            self.paragraph_cps.push(cp_lim.saturating_sub(1));
        }
        (self.document, self.paragraph_cps)
    }
}

fn add_story_capabilities(
    stories: &StoryCounts,
    features_seen: &mut BTreeSet<String>,
    unsupported: &mut BTreeSet<String>,
) {
    for (name, count) in [
        ("footnotes", stories.footnotes),
        ("headers", stories.headers),
        ("comments", stories.comments),
        ("endnotes", stories.endnotes),
        ("textboxes", stories.textboxes),
        ("header-textboxes", stories.header_textboxes),
    ] {
        if count != 0 {
            features_seen.insert(name.to_owned());
            unsupported.insert(name.to_owned());
        }
    }
}

fn decode_ansi_byte(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        other => char::from(other),
    }
}

fn require_len(data: &[u8], length: usize, field: &'static str) -> Result<(), DocError> {
    if data.len() < length {
        Err(DocError::TruncatedFib { field })
    } else {
        Ok(())
    }
}

fn read_nonnegative_i32(data: &[u8], offset: usize, field: &'static str) -> Result<u32, DocError> {
    let value = read_i32(data, offset, field)?;
    if value < 0 {
        Err(DocError::InvalidFibField {
            field,
            value: i64::from(value),
        })
    } else {
        Ok(value as u32)
    }
}

fn read_fc_lcb_pair(
    data: &[u8],
    pair_index: usize,
    field: &'static str,
) -> Result<(u32, u32), DocError> {
    let offset = FIB_FC_LCB_OFFSET
        .checked_add(pair_index * 8)
        .ok_or(DocError::TruncatedFib { field })?;
    Ok((
        read_u32(data, offset, field)?,
        read_u32(data, offset + 4, field)?,
    ))
}

fn read_u16(data: &[u8], offset: usize, field: &'static str) -> Result<u16, DocError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(DocError::TruncatedFib { field })?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize, field: &'static str) -> Result<u32, DocError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(DocError::TruncatedFib { field })?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i32(data: &[u8], offset: usize, field: &'static str) -> Result<i32, DocError> {
    Ok(read_u32(data, offset, field)? as i32)
}

fn read_u16_slice(data: &[u8], offset: usize, field: &'static str) -> Result<u16, DocError> {
    read_u16(data, offset, field).map_err(|_| DocError::MalformedClx(format!("truncated {field}")))
}

fn read_u32_slice(data: &[u8], offset: usize, field: &'static str) -> Result<u32, DocError> {
    read_u32(data, offset, field).map_err(|_| DocError::MalformedClx(format!("truncated {field}")))
}

fn read_i32_slice(data: &[u8], offset: usize, field: &'static str) -> Result<i32, DocError> {
    Ok(read_u32_slice(data, offset, field)? as i32)
}

fn read_u16_at(data: &[u8], offset: usize, field: &str) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| format!("truncated {field}"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_at(data: &[u8], offset: usize, field: &str) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| format!("truncated {field}"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i32_at(data: &[u8], offset: usize, field: &str) -> Result<i32, String> {
    Ok(read_u32_at(data, offset, field)? as i32)
}

fn u16_from(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn i16_from(bytes: &[u8]) -> i16 {
    i16::from_le_bytes([bytes[0], bytes[1]])
}

fn i32_from(bytes: &[u8]) -> i32 {
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn u32_from(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_doc(text: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let text_offset = 1024_usize;
        let mut word = vec![0_u8; text_offset + text.len()];
        word[0..2].copy_from_slice(&0xA5EC_u16.to_le_bytes());
        word[2..4].copy_from_slice(&0x00C1_u16.to_le_bytes());
        word[10..12].copy_from_slice(&(0x1000_u16 | 0x0200_u16).to_le_bytes());
        word[32..34].copy_from_slice(&0x000E_u16.to_le_bytes());
        word[62..64].copy_from_slice(&0x0016_u16.to_le_bytes());
        let word_len = word.len() as u32;
        word[64..68].copy_from_slice(&word_len.to_le_bytes());
        word[76..80].copy_from_slice(&(text.len() as u32).to_le_bytes());
        word[152..154].copy_from_slice(&0x005D_u16.to_le_bytes());
        word[418..422].copy_from_slice(&0_u32.to_le_bytes());
        word[422..426].copy_from_slice(&21_u32.to_le_bytes());
        word[text_offset..].copy_from_slice(text);

        let mut table = Vec::new();
        table.push(0x02);
        table.extend_from_slice(&16_u32.to_le_bytes());
        table.extend_from_slice(&0_i32.to_le_bytes());
        table.extend_from_slice(&(text.len() as i32).to_le_bytes());
        table.extend_from_slice(&0_u16.to_le_bytes());
        let encoded_fc = ((text_offset as u32) * 2) | 0x4000_0000;
        table.extend_from_slice(&encoded_fc.to_le_bytes());
        table.extend_from_slice(&0_u16.to_le_bytes());
        (word, table)
    }

    #[test]
    fn parses_compressed_piece_table_into_paragraphs() {
        let (word, table) = synthetic_doc(b"first\rsecond\r");
        let fib = parse_fib(&word).unwrap();
        assert_eq!(fib.table_stream, "1Table");
        let parsed = parse_document(&word, &table, &[], fib).unwrap();
        assert_eq!(parsed.piece_count, 1);
        assert_eq!(parsed.document.paragraphs.len(), 2);
        assert_eq!(
            parsed.document.paragraphs[0].children,
            vec![Inline::Text("first".to_owned())]
        );
        assert_eq!(
            parsed.document.paragraphs[1].children,
            vec![Inline::Text("second".to_owned())]
        );
    }

    #[test]
    fn rejects_encrypted_documents() {
        let (mut word, _) = synthetic_doc(b"x\r");
        let flags = u16::from_le_bytes([word[10], word[11]]) | (1 << 8);
        word[10..12].copy_from_slice(&flags.to_le_bytes());
        assert_eq!(parse_fib(&word), Err(DocError::Encrypted));
    }

    #[test]
    fn preserves_field_result_but_not_instruction() {
        let mut features = BTreeSet::new();
        let mut unsupported = BTreeSet::new();
        let mut warnings = Vec::new();
        let chars: Vec<char> = "before \u{13}PAGE\u{14}4\u{15} after\r".chars().collect();
        let symbols = vec![None; chars.len()];
        let (doc, _) = text_to_ir(
            &chars,
            &symbols,
            &TextboxPlacements::default(),
            &mut features,
            &mut unsupported,
            &mut warnings,
        );
        assert_eq!(
            doc.paragraphs[0].children,
            vec![Inline::Text("before 4 after".to_owned())]
        );
        assert!(unsupported.contains("fields"));
    }
}
