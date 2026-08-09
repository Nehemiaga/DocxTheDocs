use docxthedocs_cfb::Container;
use docxthedocs_doc::{DocError, parse_document, parse_fib};
use docxthedocs_ir::{CapabilityReport, Status, StreamKind};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default)]
pub struct ConvertOptions;

#[derive(Debug, Clone)]
pub struct ConvertResult {
    pub status: Status,
    pub report: CapabilityReport,
    pub output: PathBuf,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ConversionFailure {
    pub message: String,
    pub report: Box<CapabilityReport>,
}

pub fn convert_file(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    _options: ConvertOptions,
) -> Result<ConvertResult, ConversionFailure> {
    let input = input.as_ref();
    let output = output.as_ref();

    validate_paths(input, output)
        .map_err(|message| failure(Status::InvalidSource, String::new(), message))?;

    let source_sha256 = sha256_file(input).map_err(|error| {
        failure(
            Status::InvalidSource,
            String::new(),
            format!("could not read source DOC {}: {error}", input.display()),
        )
    })?;

    let mut container = Container::open(input).map_err(|error| {
        failure(
            Status::InvalidSource,
            source_sha256.clone(),
            format!("invalid CFB/OLE container: {error}"),
        )
    })?;
    let inventory = container.inventory().to_vec();
    let word_document = container.read_stream("/WordDocument").map_err(|error| {
        failure(
            Status::InvalidSource,
            source_sha256.clone(),
            format!("invalid Word binary document: {error}"),
        )
    })?;
    let fib = match parse_fib(&word_document) {
        Ok(fib) => fib,
        Err(DocError::Encrypted) => {
            let mut result = failure(
                Status::UnsupportedSource,
                source_sha256,
                "encrypted or obfuscated DOC files are not supported by the native converter"
                    .to_owned(),
            );
            result.report.features_seen.insert("encryption".to_owned());
            result.report.unsupported.insert("encryption".to_owned());
            result.report.stream_inventory = inventory;
            return Err(result);
        }
        Err(error) => {
            return Err(failure(
                Status::InvalidSource,
                source_sha256,
                format!("invalid FIB: {error}"),
            ));
        }
    };
    let table_stream = container
        .read_stream(&format!("/{}", fib.table_stream))
        .map_err(|error| {
            failure(
                Status::InvalidSource,
                source_sha256.clone(),
                format!("invalid table stream: {error}"),
            )
        })?;
    let data_stream = if inventory.iter().any(|entry| entry.path == "/Data") {
        container.read_stream("/Data").unwrap_or_default()
    } else {
        Vec::new()
    };
    let parsed =
        parse_document(&word_document, &table_stream, &data_stream, fib).map_err(|error| {
            failure(
                Status::InvalidSource,
                source_sha256.clone(),
                format!("invalid Word document structures: {error}"),
            )
        })?;

    let mut features_seen = parsed.features_seen;
    let mut unsupported = parsed.unsupported;
    add_container_capabilities(&inventory, &mut features_seen, &mut unsupported);
    let status = if unsupported.is_empty() {
        Status::Converted
    } else {
        Status::ConvertedWithWarnings
    };
    let report = CapabilityReport {
        engine: "DocxTheDocs".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        source_sha256,
        status,
        features_seen,
        unsupported,
        warnings: parsed.warnings,
        stories: parsed.fib.stories,
        stream_inventory: inventory,
        pieces: parsed.piece_count,
        paragraphs: parsed.document.paragraphs.len(),
    };

    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        failure_from_report(
            report.clone(),
            format!(
                "could not create output directory {}: {error}",
                parent.display()
            ),
        )
    })?;
    let temp = temporary_output_path(output);
    let mut cleanup = TempFile::new(temp.clone());
    docxthedocs_ooxml::write_docx(&temp, &parsed.document).map_err(|error| {
        failure_from_report(
            report.clone(),
            format!("could not write DOCX: {error}"),
        )
    })?;
    docxthedocs_validate::validate_docx(&temp).map_err(|error| {
        failure_from_report(
            report.clone(),
            format!("generated DOCX failed structural validation: {error}"),
        )
    })?;
    fs::rename(&temp, output).map_err(|error| {
        failure_from_report(
            report.clone(),
            format!("could not publish DOCX to {}: {error}", output.display()),
        )
    })?;
    cleanup.disarm();

    Ok(ConvertResult {
        status,
        report,
        output: output.to_path_buf(),
    })
}

fn validate_paths(input: &Path, output: &Path) -> Result<(), String> {
    if input
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        != Some("doc".to_owned())
    {
        return Err(format!(
            "source must have a .doc extension: {}",
            input.display()
        ));
    }
    if output
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        != Some("docx".to_owned())
    {
        return Err(format!(
            "destination must have a .docx extension: {}",
            output.display()
        ));
    }
    let input_absolute = fs::canonicalize(input)
        .map_err(|error| format!("could not resolve source {}: {error}", input.display()))?;
    let output_absolute = if output.exists() {
        fs::canonicalize(output).map_err(|error| {
            format!(
                "could not resolve destination {}: {error}",
                output.display()
            )
        })?
    } else {
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        parent.join(output.file_name().unwrap_or_default())
    };
    if input_absolute == output_absolute {
        return Err("source and destination must be different files".to_owned());
    }
    if output.exists() && !output.is_file() {
        return Err(format!(
            "destination exists and is not a regular file: {}",
            output.display()
        ));
    }
    Ok(())
}

fn add_container_capabilities(
    inventory: &[docxthedocs_ir::StreamInfo],
    features_seen: &mut BTreeSet<String>,
    unsupported: &mut BTreeSet<String>,
) {
    for entry in inventory {
        let lower = entry.path.to_ascii_lowercase();
        if lower.starts_with("/objectpool") {
            features_seen.insert("embedded-objects".to_owned());
            unsupported.insert("embedded-objects".to_owned());
        }
        if lower.contains("/vba") || lower.starts_with("/macros") {
            features_seen.insert("macros".to_owned());
            unsupported.insert("macros".to_owned());
        }
        if lower == "/data" && entry.kind == StreamKind::Stream && entry.size > 0 {
            features_seen.insert("data-stream".to_owned());
        }
    }
}

fn failure(status: Status, source_sha256: String, message: String) -> ConversionFailure {
    let mut report = empty_report(status, source_sha256);
    report.warnings.push(message.clone());
    ConversionFailure {
        message,
        report: Box::new(report),
    }
}

fn failure_from_report(mut report: CapabilityReport, message: String) -> ConversionFailure {
    report.status = Status::InternalError;
    report.warnings.push(message.clone());
    ConversionFailure {
        message,
        report: Box::new(report),
    }
}

fn empty_report(status: Status, source_sha256: String) -> CapabilityReport {
    CapabilityReport {
        engine: "DocxTheDocs".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        source_sha256,
        status,
        features_seen: BTreeSet::new(),
        unsupported: BTreeSet::new(),
        warnings: Vec::new(),
        stories: Default::default(),
        stream_inventory: Vec::new(),
        pieces: 0,
        paragraphs: 0,
    }
}

fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(hex)
}

fn temporary_output_path(output: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output.docx");
    output.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), sequence))
}

struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use zip::ZipArchive;

    fn write_synthetic_doc(path: &Path, text: &str) {
        let encoded: Vec<u16> = text.encode_utf16().collect();
        let text_offset = 1024_usize;
        let mut word = vec![0_u8; text_offset + encoded.len() * 2];
        word[0..2].copy_from_slice(&0xA5EC_u16.to_le_bytes());
        word[2..4].copy_from_slice(&0x00C1_u16.to_le_bytes());
        word[10..12].copy_from_slice(&(0x1000_u16 | 0x0200_u16).to_le_bytes());
        word[32..34].copy_from_slice(&0x000E_u16.to_le_bytes());
        word[62..64].copy_from_slice(&0x0016_u16.to_le_bytes());
        let word_len = word.len() as u32;
        word[64..68].copy_from_slice(&word_len.to_le_bytes());
        word[76..80].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
        word[152..154].copy_from_slice(&0x005D_u16.to_le_bytes());
        word[418..422].copy_from_slice(&0_u32.to_le_bytes());
        word[422..426].copy_from_slice(&21_u32.to_le_bytes());
        for (index, unit) in encoded.iter().enumerate() {
            let offset = text_offset + index * 2;
            word[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }

        let mut table = Vec::new();
        table.push(0x02);
        table.extend_from_slice(&16_u32.to_le_bytes());
        table.extend_from_slice(&0_i32.to_le_bytes());
        table.extend_from_slice(&(encoded.len() as i32).to_le_bytes());
        table.extend_from_slice(&0_u16.to_le_bytes());
        table.extend_from_slice(&(text_offset as u32).to_le_bytes());
        table.extend_from_slice(&0_u16.to_le_bytes());

        let mut compound = cfb::create(path).unwrap();
        compound
            .create_stream("/WordDocument")
            .unwrap()
            .write_all(&word)
            .unwrap();
        compound
            .create_stream("/1Table")
            .unwrap()
            .write_all(&table)
            .unwrap();
    }

    #[test]
    fn converts_hebrew_unicode_without_external_engine_and_is_deterministic() {
        let base = std::env::temp_dir().join(format!(
            "docxthedocs-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&base).unwrap();
        let input = base.join("input.doc");
        let output_a = base.join("a.docx");
        let output_b = base.join("b.docx");
        write_synthetic_doc(&input, "שלום עולם 123 ABC\rSecond line\r");

        let first = convert_file(&input, &output_a, ConvertOptions).unwrap();
        let _second = convert_file(&input, &output_b, ConvertOptions).unwrap();
        assert!(matches!(
            first.status,
            Status::Converted | Status::ConvertedWithWarnings
        ));
        assert_eq!(fs::read(&output_a).unwrap(), fs::read(&output_b).unwrap());
        docxthedocs_validate::validate_docx(&output_a).unwrap();

        let file = fs::File::open(&output_a).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("שלום עולם 123 ABC"));
        assert!(xml.contains("Second line"));
        assert!(xml.contains("<w:bidi"));

        fs::remove_dir_all(base).unwrap();
    }
}
