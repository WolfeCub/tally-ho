//! The text layer of a PDF invoice.
//!
//! The model takes images or text and nothing else, so a PDF has to become one
//! of them first. A vendor-generated invoice carries its own text, exact where a
//! transcription would be a guess. A scan has none and stops here.

/// How far in to look for the header. The spec allows junk in front of it, and
/// readers scan a kilobyte before giving up.
const HEADER_WINDOW: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("could not read the PDF: {0}")]
    Read(#[from] pdf_extract::OutputError),
    #[error("this PDF is a scan with no text in it: photograph it instead")]
    NoText,
}

/// Whether these bytes are a PDF, which decides both how a receipt is read and
/// where the store puts it.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(HEADER_WINDOW)]
        .windows(5)
        .any(|w| w == b"%PDF-")
}

/// The document's text, line by line as printed.
pub fn text(bytes: &[u8]) -> Result<String, PdfError> {
    let text = tidy(&pdf_extract::extract_text_from_mem(bytes)?);
    if text.is_empty() {
        return Err(PdfError::NoText);
    }
    Ok(text)
}

/// The text without its trailing spaces or blank lines.
///
/// Blank lines go entirely: the extractor emits one between every run of text,
/// so they mark nothing and are half the tokens the model would be sent.
fn tidy(raw: &str) -> String {
    raw.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_header_past_leading_junk() {
        assert!(is_pdf(b"%PDF-1.7\n"));
        assert!(is_pdf(&[&b"\n\r\n"[..], &b"%PDF-1.4"[..]].concat()));
        assert!(!is_pdf(b"\xff\xd8\xff\xe0"));
        assert!(!is_pdf(b""));
    }

    /// A header past the window is not one we'll find, and neither will lopdf.
    #[test]
    fn ignores_a_header_a_kilobyte_in() {
        let buried = [&vec![b' '; HEADER_WINDOW][..], &b"%PDF-1.4"[..]].concat();
        assert!(!is_pdf(&buried));
    }

    #[test]
    fn drops_blank_lines_and_trailing_space() {
        assert_eq!(
            tidy("\n\nInvoice   \n\n\n\nTotal  4.99\n\n\n"),
            "Invoice\nTotal  4.99"
        );
    }
}
