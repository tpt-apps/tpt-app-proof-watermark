//! Lossless metadata stripping.
//!
//! JPEG: rewrite the marker segment chain, dropping the metadata-bearing
//! segments (APP1 Exif/XMP, APP13 Photoshop/IPTC, COM comments) and copying
//! everything else — quantization/Huffman tables and scan data included —
//! through verbatim, so pixels are untouched.
//!
//! PNG: copy the chunk stream, dropping `eXIf` and the text chunks
//! (`tEXt`/`zTXt`/`iTXt`); CRCs are copied unchanged because chunk data is.
//!
//! Both paths are exercised by `engine/tests/validation.rs` with an
//! inspector that re-parses the output.

use crate::{ForgeError, ForgeResult};

/// JPEG segment markers carrying metadata. APP1 covers both Exif
/// (`Exif\0\0`) and XMP (`http://ns.adobe.com/xap/...`); APP13 carries
/// Photoshop IRB/IPTC; COM is a free-text comment.
const JPEG_DROP: &[u8] = &[0xE1, 0xED, 0xFE];

/// Strip metadata from JPEG bytes without touching compressed pixel data.
pub fn strip_jpeg_metadata(input: &[u8]) -> ForgeResult<Vec<u8>> {
    if input.len() < 4 || input[0] != 0xFF || input[1] != 0xD8 {
        return Err(ForgeError::Decode("not a JPEG (missing SOI)".into()));
    }
    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(&input[0..2]); // SOI

    let mut pos = 2usize;
    loop {
        // Segments begin with 0xFF; tolerate fill bytes (0xFF padding).
        while pos + 1 < input.len() && input[pos] == 0xFF && input[pos + 1] == 0xFF {
            pos += 1;
        }
        if pos + 1 >= input.len() || input[pos] != 0xFF {
            return Err(ForgeError::Decode("corrupt JPEG: expected a marker".into()));
        }
        let marker = input[pos + 1];
        if marker == 0xD9 {
            // EOI — end of the chain (single-image JPEGs end here).
            out.extend_from_slice(&input[pos..pos + 2]);
            pos += 2;
            break;
        }
        if marker == 0xDA {
            // Start of Scan: everything from here on is entropy-coded data.
            out.extend_from_slice(&input[pos..]);
            return Ok(out);
        }
        // Standalone markers without a length field.
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            out.extend_from_slice(&input[pos..pos + 2]);
            pos += 2;
            continue;
        }
        // Length-prefixed segment.
        if pos + 4 > input.len() {
            return Err(ForgeError::Decode("corrupt JPEG: truncated segment header".into()));
        }
        let len = u16::from_be_bytes([input[pos + 2], input[pos + 3]]) as usize;
        if len < 2 || pos + 2 + len > input.len() {
            return Err(ForgeError::Decode("corrupt JPEG: bad segment length".into()));
        }
        let seg_end = pos + 2 + len;
        if JPEG_DROP.contains(&marker) {
            pos = seg_end;
            continue;
        }
        out.extend_from_slice(&input[pos..seg_end]);
        pos = seg_end;
    }
    // Trailing bytes after EOI (non-standard but seen in the wild): keep them.
    if pos < input.len() {
        out.extend_from_slice(&input[pos..]);
    }
    Ok(out)
}

/// Strip `eXIf` and text chunks from PNG bytes.
pub fn strip_png_metadata(input: &[u8]) -> ForgeResult<Vec<u8>> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if input.len() < 8 || &input[0..8] != SIG {
        return Err(ForgeError::Decode("not a PNG (missing signature)".into()));
    }
    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(SIG);

    let mut pos = 8usize;
    while pos + 12 <= input.len() {
        let len = u32::from_be_bytes([input[pos], input[pos + 1], input[pos + 2], input[pos + 3]])
            as usize;
        let data_end = pos + 8 + len;
        if data_end + 4 > input.len() {
            return Err(ForgeError::Decode("corrupt PNG: truncated chunk".into()));
        }
        let ctype = &input[pos + 4..pos + 8];
        let drop = matches!(ctype, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt");
        if !drop {
            out.extend_from_slice(&input[pos..data_end + 4]);
        }
        pos = data_end + 4;
        if ctype == b"IEND" {
            break;
        }
    }
    if pos < input.len() {
        // Keep anything after IEND rather than failing on it.
        out.extend_from_slice(&input[pos..]);
    }
    Ok(out)
}

// --- inspector helpers ---
//
// Public (not cfg(test)) so the integration tests in engine/tests/ — and the
// desktop shell, if it ever wants a "what's left" readout — can re-parse a
// file and list what survived stripping.

/// List the JPEG marker segments remaining in `bytes` (up to SOS), as
/// `(marker, length)` pairs.
pub fn jpeg_segments(bytes: &[u8]) -> Vec<(u8, usize)> {
    let mut segs = Vec::new();
    let mut pos = 2usize;
    while pos + 1 < bytes.len() && bytes[pos] == 0xFF {
        let marker = bytes[pos + 1];
        if marker == 0xD9 || marker == 0xDA || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            segs.push((marker, 0));
            if marker == 0xDA || marker == 0xD9 {
                break;
            }
            pos += 2;
            continue;
        }
        if pos + 4 > bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        segs.push((marker, len));
        pos += 2 + len;
    }
    segs
}

/// List the PNG chunk types remaining in `bytes`.
pub fn png_chunks(bytes: &[u8]) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    let mut pos = 8usize;
    while pos + 12 <= bytes.len() {
        let len =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let mut ctype = [0u8; 4];
        ctype.copy_from_slice(&bytes[pos + 4..pos + 8]);
        out.push(ctype);
        pos += 8 + len + 4;
        if &ctype == b"IEND" {
            break;
        }
    }
    out
}
