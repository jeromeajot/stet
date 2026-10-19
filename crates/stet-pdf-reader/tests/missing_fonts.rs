// stet - A PostScript Interpreter
// Copyright (c) 2026 Scott Bowman
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A font or a CMap the document uses without carrying it, and that the
//! reader does not carry either, is reported as its own kind,
//! `ParsePhase::MissingFont` or `ParsePhase::MissingCMap`, for the page
//! that uses it and without its name. A missing font is drawn with a
//! substitute and its text is still read; text in a font whose CMap is
//! missing is left out, since without the CMap its codes cannot be read.
//!
//! Predefined CMaps are looked for in `STET_CMAP_DIR` first. Every test
//! here points it at an empty folder of its own, so that the CMaps a
//! machine may have installed there are not found.

use std::sync::Once;

use stet_graphics::text::text_runs;
use stet_pdf_reader::{
    LayerSet, LocationHint, ParsePhase, ParseWarning, PdfDocument, Severity, TextExtraction,
};

/// Point `STET_CMAP_DIR` at an empty folder, once for the whole binary.
fn no_cmap_files() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-cmap-files");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            std::fs::read_dir(&dir).unwrap().next().is_none(),
            "{} must be empty",
            dir.display()
        );
        // SAFETY: every test in this binary calls this before it reads a
        // document, and the variable is read through `std::env` alone,
        // which serializes access to the environment.
        unsafe { std::env::set_var("STET_CMAP_DIR", &dir) };
    });
}

/// A PDF of `objects`, numbered from 1, with a cross-reference table.
fn pdf_from(objects: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for off in offsets {
        pdf.extend(format!("{off:010} 00000 n\r\n").as_bytes());
    }
    pdf.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn stream(dict: &str, body: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{body}\nendstream",
        body.len()
    )
}

/// Two pages that both run `content`, with fonts `/F1` (Helvetica, which
/// the reader carries) and `/F2` (object 7, `font`), and `extra` objects
/// from 8 up.
fn two_pages(font: &str, content: &str, extra: &[String]) -> Vec<u8> {
    let resources = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> \
                     /F2 7 0 R >>";
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 100] \
             /Resources << {resources} >> /Contents {contents} 0 R >>"
        )
    };
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_string(),
        page(4),
        stream("", content),
        page(6),
        stream("", content),
        font.to_string(),
    ];
    objects.extend_from_slice(extra);
    pdf_from(&objects)
}

/// A composite font over a CIDFontType2 that is not embedded (object 8),
/// encoded with the CMap `encoding`. Its `/ToUnicode` (object 10) gives
/// the text of the two GB 2312 codes in [`MIXED`], so that text read from
/// codes guessed to be two bytes long would be found.
fn composite_font(encoding: &str) -> (String, Vec<String>) {
    let font = format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /{encoding} \
         /DescendantFonts [8 0 R] /ToUnicode 10 0 R >>"
    );
    let to_unicode = stream(
        "",
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CMapName /Test def\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
         2 beginbfchar\n<B0A1> <554A>\n<B0A2> <963F>\nendbfchar\n\
         endcmap CMapName currentdict /CMap defineresource pop end end",
    );
    let descendant = "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SimSun \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> \
         /FontDescriptor 9 0 R /DW 1000 >>"
        .to_string();
    let descriptor = "<< /Type /FontDescriptor /FontName /SimSun /Flags 6 \
         /FontBBox [0 -141 996 855] /ItalicAngle 0 /Ascent 859 /Descent -141 \
         /CapHeight 700 /StemV 80 >>"
        .to_string();
    (font, vec![descendant, descriptor, to_unicode])
}

/// Latin text in Helvetica, then text in `/F2`: two GB 2312 characters
/// (four bytes, each pair a character under GBK-EUC-H) and, under the same
/// CMap, the one-byte codes of "SSN".
const MIXED: &str = "BT /F1 12 Tf 10 60 Td (Latin value) Tj ET \
                     BT /F2 12 Tf 10 30 Td <B0A1B0A2> Tj (SSN) Tj ET";

/// Each page's text, and the document's warnings, after both pages are
/// rendered twice.
fn read(pdf: &[u8]) -> (Vec<String>, Vec<ParseWarning>) {
    no_cmap_files();
    let mut doc = PdfDocument::from_bytes(pdf).unwrap();
    doc.set_text_extraction(TextExtraction::Runs);
    let mut texts = Vec::new();
    for round in 0..2 {
        for page in 0..doc.page_count() {
            let list = doc.render_page(page, 72.0).unwrap();
            let text: String = text_runs(&list, &LayerSet::new())
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            if round == 0 {
                texts.push(text);
            }
        }
    }
    let warnings = doc.parse_warnings().to_vec();
    (texts, warnings)
}

/// The warnings of `phase`, as `(page, severity)`.
fn of_kind(warnings: &[ParseWarning], phase: ParsePhase) -> Vec<(Option<usize>, Severity)> {
    warnings
        .iter()
        .filter(|w| w.phase == phase)
        .map(|w| {
            let page = match w.location {
                Some(LocationHint::Page(page)) => Some(page),
                None => None,
                ref other => panic!("unexpected location {other:?}"),
            };
            (page, w.severity)
        })
        .collect()
}

fn both_pages() -> Vec<(Option<usize>, Severity)> {
    vec![(Some(0), Severity::Warning), (Some(1), Severity::Warning)]
}

/// The folders other than `STET_CMAP_DIR` the reader looks in for a
/// predefined CMap. The test below needs GBK-EUC-H to be in none of them.
fn installed_cmap(name: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut places: Vec<std::path::PathBuf> = Vec::new();
    if let Some(home) = home {
        places.push(home.join(".local/share/stet/CMap").join(name));
    }
    for base in [
        "/usr/share/poppler/cMap",
        "/usr/local/share/poppler/cMap",
        "/opt/homebrew/share/poppler/cMap",
        "/usr/local/opt/poppler-data/share/poppler/cMap",
    ] {
        places.push(std::path::Path::new(base).join("Adobe-GB1").join(name));
    }
    for dir in [
        "/var/lib/ghostscript/CMap",
        "/usr/share/ghostscript/Resource/CMap",
        "/usr/local/share/ghostscript/Resource/CMap",
    ] {
        places.push(std::path::Path::new(dir).join(name));
    }
    places.into_iter().find(|p| p.exists())
}

/// GBK-EUC-H, a predefined CMap, is not embedded and not found. Each page
/// reports the CMap as missing, and the font too, since it is not embedded
/// either. Nothing in that font is drawn or read, not even the one-byte
/// Latin codes, and nothing is a content warning. The Helvetica text on
/// the same page is read as usual.
#[test]
fn a_predefined_cmap_that_is_not_found_is_its_own_kind_and_its_text_is_left_out() {
    if let Some(found) = installed_cmap("GBK-EUC-H") {
        panic!(
            "GBK-EUC-H is installed at {}, where the reader finds it; \
             this test needs a machine without it",
            found.display()
        );
    }
    let (font, extra) = composite_font("GBK-EUC-H");
    let pdf = two_pages(&font, MIXED, &extra);
    let (texts, warnings) = read(&pdf);

    assert_eq!(of_kind(&warnings, ParsePhase::MissingCMap), both_pages());
    assert_eq!(of_kind(&warnings, ParsePhase::MissingFont), both_pages());
    assert_eq!(of_kind(&warnings, ParsePhase::Content), []);
    assert_eq!(texts, ["Latin value", "Latin value"]);
    for w in &warnings {
        for name in ["GBK", "SimSun", "F2"] {
            assert!(!w.message.contains(name), "{w:?} names {name}");
        }
    }

    // Nothing is drawn in that font either: the page draws what the
    // Helvetica line alone draws.
    let (font_only_latin, extra_latin) = composite_font("GBK-EUC-H");
    let latin_only = two_pages(
        &font_only_latin,
        "BT /F1 12 Tf 10 60 Td (Latin value) Tj ET",
        &extra_latin,
    );
    no_cmap_files();
    let elements = |pdf: &[u8]| {
        let doc = PdfDocument::from_bytes(pdf).unwrap();
        doc.render_page(0, 72.0).unwrap().elements().len()
    };
    assert_eq!(elements(&pdf), elements(&latin_only));
}

/// A CMap name that no machine has is reported the same way.
#[test]
fn a_cmap_no_machine_has_is_its_own_kind() {
    let (font, extra) = composite_font("NoSuchCMap-H");
    let (texts, warnings) = read(&two_pages(&font, MIXED, &extra));
    assert_eq!(of_kind(&warnings, ParsePhase::MissingCMap), both_pages());
    assert_eq!(of_kind(&warnings, ParsePhase::Content), []);
    assert_eq!(texts, ["Latin value", "Latin value"]);
}

/// Identity-H is the reader's own and is never reported; the font, which
/// is not embedded, still is.
#[test]
fn identity_h_is_never_reported_as_missing() {
    let (font, extra) = composite_font("Identity-H");
    let (_, warnings) = read(&two_pages(
        &font,
        "BT /F2 12 Tf 10 30 Td <00240025> Tj ET",
        &extra,
    ));
    assert_eq!(of_kind(&warnings, ParsePhase::MissingCMap), []);
    assert_eq!(of_kind(&warnings, ParsePhase::MissingFont), both_pages());
    assert_eq!(of_kind(&warnings, ParsePhase::Content), []);
}

/// A simple font that is not embedded is drawn with a substitute, and its
/// text is read. It is reported for each page as a missing font, and not
/// by name.
#[test]
fn a_font_that_is_not_embedded_is_its_own_kind_and_its_text_is_read() {
    let font = "<< /Type /Font /Subtype /TrueType /BaseFont /Arial,Bold \
                /FirstChar 32 /LastChar 126 /Encoding /WinAnsiEncoding \
                /FontDescriptor 8 0 R >>";
    let descriptor = "<< /Type /FontDescriptor /FontName /Arial,Bold /Flags 32 \
                      /FontBBox [-628 -376 2000 1056] /ItalicAngle 0 /Ascent 905 \
                      /Descent -212 /CapHeight 716 /StemV 153 >>"
        .to_string();
    let (texts, warnings) = read(&two_pages(
        font,
        "BT /F2 12 Tf 10 30 Td (Account 1234) Tj ET",
        &[descriptor],
    ));
    assert_eq!(of_kind(&warnings, ParsePhase::MissingFont), both_pages());
    assert_eq!(of_kind(&warnings, ParsePhase::Content), []);
    assert_eq!(texts, ["Account 1234", "Account 1234"]);
    for w in &warnings {
        assert!(!w.message.contains("Arial"), "{w:?}");
    }
}

/// The standard 14 fonts are the reader's own, and a subset tag does not
/// hide one.
#[test]
fn a_standard_14_font_is_never_reported_as_missing() {
    for name in [
        "Times-Roman",
        "ABCDEF+Courier-Bold",
        "Helvetica,Bold",
        "Symbol",
    ] {
        let font = format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>");
        let (_, warnings) = read(&two_pages(&font, "BT /F2 12 Tf 10 30 Td (x) Tj ET", &[]));
        assert!(warnings.is_empty(), "{name}: {warnings:?}");
    }
}

/// A font the document embeds but whose program cannot be read is not a
/// missing font: it is a fault in the file, and a content warning.
#[test]
fn an_embedded_font_that_cannot_be_read_is_a_content_warning() {
    let font = "<< /Type /Font /Subtype /TrueType /BaseFont /Broken \
                /FirstChar 32 /LastChar 126 /Encoding /WinAnsiEncoding \
                /FontDescriptor 8 0 R >>";
    let descriptor = "<< /Type /FontDescriptor /FontName /Broken /Flags 32 \
                      /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 900 \
                      /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 9 0 R >>"
        .to_string();
    let program = stream("", "this is not a font program");
    let (_, warnings) = read(&two_pages(
        font,
        "BT /F2 12 Tf 10 30 Td (x) Tj ET",
        &[descriptor, program],
    ));
    assert_eq!(of_kind(&warnings, ParsePhase::MissingFont), []);
    assert_eq!(of_kind(&warnings, ParsePhase::Content), both_pages());
}
