use cfb::CompoundFile;
use docxthedocs_ir::{StreamInfo, StreamKind};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use thiserror::Error;

const MAX_CONTAINER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_STREAM_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STREAMS: usize = 16_384;

#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("could not inspect source file: {0}")]
    Io(#[from] io::Error),
    #[error("source is too large ({actual} bytes; limit is {limit} bytes)")]
    ContainerTooLarge { actual: u64, limit: u64 },
    #[error("compound file contains too many entries (limit is {0})")]
    TooManyEntries(usize),
    #[error("stream {path} is too large ({actual} bytes; limit is {limit} bytes)")]
    StreamTooLarge {
        path: String,
        actual: u64,
        limit: u64,
    },
    #[error("required stream is missing: {0}")]
    MissingStream(String),
}

pub struct Container {
    compound: CompoundFile<File>,
    inventory: Vec<StreamInfo>,
}

impl Container {
    pub fn open(path: &Path) -> Result<Self, ContainerError> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(ContainerError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source is not a regular file",
            )));
        }
        if metadata.len() > MAX_CONTAINER_BYTES {
            return Err(ContainerError::ContainerTooLarge {
                actual: metadata.len(),
                limit: MAX_CONTAINER_BYTES,
            });
        }

        let compound = cfb::OpenOptions::new()
            .max_buffer_size(1024 * 1024)
            .open(path)?;
        let mut inventory = Vec::new();
        for entry in compound.walk() {
            if inventory.len() >= MAX_STREAMS {
                return Err(ContainerError::TooManyEntries(MAX_STREAMS));
            }
            let kind = if entry.is_root() {
                StreamKind::Root
            } else if entry.is_storage() {
                StreamKind::Storage
            } else {
                StreamKind::Stream
            };
            inventory.push(StreamInfo {
                path: entry.path().to_string_lossy().into_owned(),
                kind,
                size: entry.len(),
            });
        }
        inventory.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(Self {
            compound,
            inventory,
        })
    }

    pub fn inventory(&self) -> &[StreamInfo] {
        &self.inventory
    }

    pub fn has_path(&self, path: &str) -> bool {
        self.compound.exists(path)
    }

    pub fn read_stream(&mut self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let entry = self
            .compound
            .entry(path)
            .map_err(|_| ContainerError::MissingStream(path.to_owned()))?;
        if !entry.is_stream() {
            return Err(ContainerError::MissingStream(path.to_owned()));
        }
        if entry.len() > MAX_STREAM_BYTES {
            return Err(ContainerError::StreamTooLarge {
                path: path.to_owned(),
                actual: entry.len(),
                limit: MAX_STREAM_BYTES,
            });
        }

        let capacity =
            usize::try_from(entry.len()).map_err(|_| ContainerError::StreamTooLarge {
                path: path.to_owned(),
                actual: entry.len(),
                limit: MAX_STREAM_BYTES,
            })?;
        let mut bytes = Vec::with_capacity(capacity);
        self.compound.open_stream(path)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
