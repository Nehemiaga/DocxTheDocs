# Hebrew and RTL conversion contract

DocxTheDocs treats Hebrew/RTL fidelity as a core conversion requirement rather than a post-processing step.

## Text

DOC text is reconstructed from the Word binary piece table. Unicode pieces are decoded as UTF-16, while compressed pieces are decoded according to the legacy DOC representation. The converter emits normal Unicode text into WordprocessingML; it never reverses Hebrew strings to simulate RTL display.

## Paragraph direction

Direction is resolved in this order:

1. Preserve an explicit paragraph bidi property recovered from the DOC formatting structures.
2. If no explicit bidi value exists, infer RTL conservatively when the paragraph contains strong Hebrew or Arabic text.
3. Leave paragraphs without strong RTL evidence unchanged.

The OOXML writer represents RTL paragraphs with `w:bidi`.

## Mixed Hebrew and Latin text

A paragraph may contain Hebrew, numbers, punctuation and Latin text. The converter preserves logical Unicode order and relies on the Unicode bidi algorithm in DOCX consumers. It does not reorder runs merely because the paragraph is RTL.

## Indentation and alignment

DOC paragraph indentation is mapped to logical OOXML start/end values so the same IR works correctly for both LTR and RTL paragraphs. First-line and hanging indentation are preserved separately. Alignment values are preserved when available.

## Tables

When DOC table properties identify RTL visual ordering, the generated table uses `w:bidiVisual` and the reconstructed cell order follows the source table semantics.

## Sections

Section-level bidi and RTL-gutter properties are preserved when present, together with page size, orientation and margins.

## Numbering

The numbering writer supports decimal, Roman, alphabetic, Hebrew and bullet labels. Paragraph-level numbering references are emitted through `numbering.xml`.

## Validation expectations

A Hebrew-focused conversion should be considered correct only when all of the following hold:

- Hebrew text is Unicode-identical after normal document-format normalization.
- Paragraph direction is correct.
- Mixed Hebrew/Latin segments remain in logical order.
- List labels remain attached to the correct paragraphs.
- RTL tables preserve their visual cell ordering.
- Logical indentation and alignment match the source intent.
- The generated DOCX passes ZIP/XML structural validation.

## Known limitations

Style inheritance is not yet fully resolved. When direction or formatting exists only through an unresolved style, the converter may use conservative text-based RTL inference and report a warning. Full images/OfficeArt, headers/footers, notes and complex multi-section placement are also incomplete and are explicitly reported rather than hidden.
