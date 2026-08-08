# DocxTheDocs

A focused, native Rust library and CLI for converting legacy Microsoft Word `.doc` files to `.docx`.

This repository has one job: **DOC → DOCX conversion**. It does not depend on Microsoft Word, LibreOffice, Python, a server, or any external conversion service.

Hebrew and RTL documents are first-class targets.

## Design goals

- Native Rust conversion with no subprocesses or office-suite dependency.
- Fast, deterministic conversion suitable for batch and parallel workloads.
- Bounded parsing of OLE/CFB and Word 97–2003 binary structures.
- Correct Unicode reconstruction from compressed and UTF-16 text pieces.
- First-class Hebrew/Arabic RTL handling.
- Atomic output: a destination is published only after DOCX ZIP/XML validation succeeds.
- No network access and no telemetry.

## Build

```bash
cargo build --release -p docxthedocs
```

The binary is produced at:

```text
target/release/docxthedocs
```

## CLI

```bash
docxthedocs convert input.doc output.docx
```

Optional machine-readable conversion report:

```bash
docxthedocs convert input.doc output.docx --report report.json
```

## Library API

```rust
use docxthedocs::{convert_file, ConvertOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let result = convert_file(
        "input.doc",
        "output.docx",
        ConvertOptions,
    )?;

    println!("status: {:?}", result.status);
    Ok(())
}
```

## Hebrew / RTL support

The converter currently preserves or reconstructs the RTL properties that matter most for Hebrew legal and administrative documents:

- Hebrew Unicode text from both compressed DOC pieces and UTF-16 pieces.
- Paragraph bidi/RTL flags when stored directly in DOC paragraph properties.
- Conservative RTL inference for Hebrew/Arabic paragraphs when the source text is RTL but a direct bidi property is absent.
- Logical start/end indentation, first-line and hanging indents.
- Right/center/justified paragraph alignment.
- Section bidi and RTL gutter properties.
- RTL table visual ordering.
- Hebrew, decimal, Roman, alphabetic and bullet numbering.
- Legacy Symbol glyph preservation through OOXML `w:sym`.
- Mixed Hebrew/Latin text without reversing the Unicode text itself.

See [`docs/HEBREW_RTL.md`](docs/HEBREW_RTL.md) for the RTL conversion contract.

## Architecture

The implementation is deliberately split into small native crates:

- `docxthedocs-cfb` — safe OLE/CFB container inventory and stream access.
- `docxthedocs-doc` — Word 97–2003 binary parser and DOC → neutral IR conversion.
- `docxthedocs-ir` — document intermediate representation.
- `docxthedocs-ooxml` — deterministic WordprocessingML/DOCX writer.
- `docxthedocs-validate` — ZIP/XML package validation.
- `docxthedocs` — public library API and CLI.

The conversion path is:

```text
DOC file
  ↓
CFB/OLE streams
  ↓
FIB + piece table + formatting structures
  ↓
neutral document IR
  ↓
WordprocessingML
  ↓
validated DOCX
```

## Conversion status

The API and optional JSON report use native-only statuses:

| Status | Meaning | CLI exit |
| --- | --- | ---: |
| `CONVERTED` | Conversion completed without known unsupported source features. | 0 |
| `CONVERTED_WITH_WARNINGS` | A valid DOCX was produced, but some source features could not be fully preserved. | 10 |
| `UNSUPPORTED_SOURCE` | The input is a DOC structure the native converter intentionally does not handle, such as encryption. | 20 |
| `INVALID_SOURCE` | The input is malformed or not a supported DOC file. | 21 |
| `INTERNAL_ERROR` | Output generation or validation failed. | 30 |

A warning status still writes a structurally valid DOCX. Callers that require strict fidelity should inspect `report.unsupported` before accepting the result.

## Current scope

Implemented today:

- CFB/OLE stream reading.
- FIB parsing and `0Table` / `1Table` selection.
- CLX / `PlcPcd` piece-table parsing.
- Main-story text reconstruction.
- Direct paragraph formatting.
- Page layout and section RTL properties.
- Automatic numbering.
- Native table reconstruction.
- Borders, shading, hidden-run suppression and legacy symbol glyphs.
- Best-effort textbox text recovery.
- Deterministic DOCX generation and structural validation.

Not yet fully preserved:

- complete style inheritance;
- headers and footers;
- footnotes/endnotes/comments;
- full embedded pictures and OfficeArt;
- complex textbox-to-shape placement;
- full multi-section placement.

These are reported as unsupported or warnings rather than silently invoking another conversion engine.

## Performance model

The converter does not launch GUI applications or subprocesses. Each conversion is independent, so callers can parallelize across files according to available CPU, memory and storage bandwidth. Source hashing is streamed and does not allocate a second full-file buffer.

For very large batches, prefer a bounded worker pool rather than unbounded task creation.

## Test

```bash
cargo test --workspace
```

The Rust test suite includes Unicode/Hebrew conversion coverage and deterministic-output checks.

## License

MIT.
