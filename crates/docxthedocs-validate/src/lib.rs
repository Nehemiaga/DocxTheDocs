use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use thiserror::Error;
use zip::ZipArchive;

const MAX_PARTS: usize = 16_384;
const MAX_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
const REQUIRED_PARTS: [&str; 5] = [
    "[Content_Types].xml",
    "_rels/.rels",
    "word/document.xml",
    "word/styles.xml",
    "word/_rels/document.xml.rels",
];

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("DOCX I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid DOCX ZIP: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("DOCX contains too many parts ({actual}; limit {limit})")]
    TooManyParts { actual: usize, limit: usize },
    #[error("DOCX expands beyond the validation limit of {0} bytes")]
    ExpandedSize(u64),
    #[error("DOCX is missing required part {0}")]
    MissingPart(&'static str),
    #[error("unsafe DOCX part name: {0}")]
    UnsafePartName(String),
    #[error("XML part {part} is malformed: {message}")]
    MalformedXml { part: String, message: String },
    #[error("XML part {part} contains a forbidden DTD")]
    Dtd { part: String },
    #[error("XML part {part} has root {actual}, expected {expected}")]
    WrongRoot {
        part: String,
        actual: String,
        expected: &'static str,
    },
}

pub fn validate_docx(path: &Path) -> Result<(), ValidationError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > MAX_PARTS {
        return Err(ValidationError::TooManyParts {
            actual: archive.len(),
            limit: MAX_PARTS,
        });
    }

    let mut names = BTreeSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") || name.contains('\\')
        {
            return Err(ValidationError::UnsafePartName(name));
        }
        total = total
            .checked_add(entry.size())
            .ok_or(ValidationError::ExpandedSize(MAX_UNCOMPRESSED_BYTES))?;
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(ValidationError::ExpandedSize(total));
        }
        names.insert(name);
    }
    for required in REQUIRED_PARTS {
        if !names.contains(required) {
            return Err(ValidationError::MissingPart(required));
        }
    }

    validate_xml_part(&mut archive, "[Content_Types].xml", "Types")?;
    validate_xml_part(&mut archive, "_rels/.rels", "Relationships")?;
    validate_xml_part(&mut archive, "word/document.xml", "w:document")?;
    validate_xml_part(&mut archive, "word/styles.xml", "w:styles")?;
    validate_xml_part(
        &mut archive,
        "word/_rels/document.xml.rels",
        "Relationships",
    )?;
    Ok(())
}

fn validate_xml_part(
    archive: &mut ZipArchive<File>,
    part: &str,
    expected_root: &'static str,
) -> Result<(), ValidationError> {
    let mut xml = Vec::new();
    archive.by_name(part)?.read_to_end(&mut xml)?;
    let mut reader = Reader::from_reader(xml.as_slice());
    reader.config_mut().check_end_names = true;
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) | Ok(Event::Empty(start)) if !saw_root => {
                let actual = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                if actual != expected_root {
                    return Err(ValidationError::WrongRoot {
                        part: part.to_owned(),
                        actual,
                        expected: expected_root,
                    });
                }
                saw_root = true;
            }
            Ok(Event::DocType(_)) => {
                return Err(ValidationError::Dtd {
                    part: part.to_owned(),
                });
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(ValidationError::MalformedXml {
                    part: part.to_owned(),
                    message: error.to_string(),
                });
            }
        }
    }
    if !saw_root {
        return Err(ValidationError::MalformedXml {
            part: part.to_owned(),
            message: "missing document element".to_owned(),
        });
    }
    Ok(())
}
