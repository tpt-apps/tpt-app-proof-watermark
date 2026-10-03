//! Engine validation: watermark rendering across image sizes/aspect ratios,
//! and EXIF-strip verification through an independent metadata inspector.

use tpt_proof_engine::exif;
use tpt_proof_engine::spec::{Color, Position, WatermarkContent, WatermarkSpec};
use tpt_proof_engine::{
    apply_watermark, decode, detect_format, encode, strip_metadata, ImageBuffer, ImageFormat,
};

/// A deterministic gray test photo with a checker pattern so untouched vs
/// watermarked regions are distinguishable.
fn test_photo(width: u32, height: u32) -> Vec<u8> {
    let mut img = ImageBuffer::new(width, height).unwrap();
    for y in 0..height {
        for x in 0..width {
            let v = if (x / 16 + y / 16) % 2 == 0 { 40u8 } else { 90 };
            img.set_pixel(x, y, [v, v, v, 255]);
        }
    }
    encode(&img, ImageFormat::Jpeg, 95).unwrap()
}

fn png_logo() -> Vec<u8> {
    let mut logo = ImageBuffer::new(40, 20).unwrap();
    for y in 0..20 {
        for x in 0..40 {
            logo.set_pixel(x, y, [255, 0, 0, 255]);
        }
    }
    encode(&logo, ImageFormat::Png, 0).unwrap()
}

fn text_spec() -> WatermarkSpec {
    WatermarkSpec {
        content: WatermarkContent::Text {
            text: "\u{A9} PROOF".to_string(),
            color: Color::WHITE,
        },
        position: Position::BottomRight,
        scale: 0.05,
        opacity: 1.0,
        tile: false,
        tile_gap: 0.25,
    }
}

/// Count pixels that differ meaningfully between two same-size images
/// (decoded back to RGBA; small codec jitter ignored).
fn diff_pixels(a: &[u8], b: &[u8], threshold: i32) -> usize {
    let (da, db) = (decode(a).unwrap(), decode(b).unwrap());
    assert_eq!((da.width, da.height), (db.width, db.height));
    da.data
        .chunks_exact(4)
        .zip(db.data.chunks_exact(4))
        .filter(|(p, q)| {
            p.iter().zip(q.iter()).any(|(x, y)| (i32::from(*x) - i32::from(*y)).abs() > threshold)
        })
        .count()
}

#[test]
fn text_watermark_renders_across_sizes_and_aspects() {
    // Portrait, landscape, square, panorama, tiny — every shape the UI will
    // meet, including the odd sizes phones produce.
    for (w, h) in [(3000u32, 2000u32), (1080, 1920), (1500, 1500), (4000, 1000), (160, 120)] {
        let photo = test_photo(w, h);
        let out = apply_watermark(&photo, &text_spec(), None, ImageFormat::Jpeg, 90)
            .unwrap_or_else(|e| panic!("{w}x{h}: {e}"));
        assert_eq!(detect_format(&out), Some(ImageFormat::Jpeg));
        let decoded = decode(&out).unwrap();
        assert_eq!((decoded.width, decoded.height), (w, h), "dimensions must be preserved");
        // The watermark must draw something, and must not bleed into the
        // top-left quarter away from its bottom-right placement.
        let changed = diff_pixels(&photo, &out, 24);
        assert!(changed > 0, "{w}x{h}: watermark drew nothing");
        let (da, db) = (decode(&photo).unwrap(), decode(&out).unwrap());
        let (mw, mh) = ((w / 4).max(1), (h / 4).max(1));
        let bled = (0..mh).any(|y| {
            (0..mw).any(|x| {
                let (p, q) = (da.get_pixel(x, y), db.get_pixel(x, y));
                p.iter().zip(q.iter()).any(|(a, b)| (i32::from(*a) - i32::from(*b)).abs() > 24)
            })
        });
        assert!(!bled, "{w}x{h}: watermark bled outside its position");
    }
}

#[test]
fn copyright_glyph_is_not_a_space() {
    // The dedicated © glyph must put ink on pixels; it would silently
    // collapse to a blank in a pure-ASCII font.
    let overlay = tpt_proof_engine::render::text_overlay("\u{A9}", 8, [255, 255, 255]);
    let ink = overlay.data.chunks_exact(4).filter(|p| p[3] == 255).count();
    assert!(ink >= 10, "© glyph drew {ink} pixels, expected a visible mark");
}

#[test]
fn opacity_scales_watermark_strength() {
    let photo = test_photo(800, 600);
    let mut spec = text_spec();
    spec.opacity = 0.0;
    let invisible = apply_watermark(&photo, &spec, None, ImageFormat::Jpeg, 95).unwrap();
    spec.opacity = 1.0;
    let opaque = apply_watermark(&photo, &spec, None, ImageFormat::Jpeg, 95).unwrap();
    // opacity 0 leaves the photo alone; opacity 1 marks plenty of pixels.
    assert_eq!(diff_pixels(&photo, &invisible, 24), 0, "opacity 0 must be a no-op");
    assert!(diff_pixels(&photo, &opaque, 24) > 50, "opacity 1 must draw");
}

#[test]
fn logo_watermark_scales_to_image() {
    let photo = test_photo(1200, 800);
    let mut spec = text_spec();
    spec.content = WatermarkContent::Logo;
    spec.scale = 0.25; // logo ~200 px wide on the 800 px short side
    let out = apply_watermark(&photo, &spec, Some(&png_logo()), ImageFormat::Jpeg, 95).unwrap();
    assert!(diff_pixels(&photo, &out, 24) > 100, "logo must draw");

    // Missing logo bytes for a Logo spec is a clean error, not a panic.
    assert!(apply_watermark(&photo, &spec, None, ImageFormat::Jpeg, 95).is_err());
}

#[test]
fn tiling_covers_multiple_quadrants() {
    let photo = test_photo(1600, 1200);
    let mut spec = text_spec();
    spec.tile = true;
    spec.tile_gap = 0.1;
    let out = apply_watermark(&photo, &spec, None, ImageFormat::Jpeg, 95).unwrap();
    let (da, db) = (decode(&photo).unwrap(), decode(&out).unwrap());
    // At least one clearly-changed pixel in each quadrant.
    let mut per_quad = [0usize; 4];
    for y in 0..da.height {
        for x in 0..da.width {
            let (p, q) = (da.get_pixel(x, y), db.get_pixel(x, y));
            let changed = p.iter().zip(q.iter()).any(|(a, b)| {
                (i32::from(*a) - i32::from(*b)).abs() > 24
            });
            if changed {
                let qx = usize::from(x >= da.width / 2);
                let qy = usize::from(y >= da.height / 2);
                per_quad[qy * 2 + qx] += 1;
            }
        }
    }
    for (i, n) in per_quad.iter().enumerate() {
        assert!(*n > 10, "quadrant {i} got no tiles ({per_quad:?})");
    }
}

#[test]
fn png_round_trip_with_transparent_regions() {
    // A PNG with alpha: the source-over composite must not trash the
    // transparent background.
    let mut img = ImageBuffer::new(400, 400).unwrap();
    for y in 0..400 {
        for x in 0..400 {
            img.set_pixel(x, y, [10, 20, 30, if x < 200 { 0 } else { 255 }]);
        }
    }
    let photo = encode(&img, ImageFormat::Png, 0).unwrap();
    let out = apply_watermark(&photo, &text_spec(), None, ImageFormat::Png, 0).unwrap();
    let decoded = decode(&out).unwrap();
    // Untouched transparent top-left keeps alpha 0.
    assert_eq!(decoded.get_pixel(10, 10)[3], 0, "transparent region must stay transparent");
}

// --------------------------------------------------------------- EXIF strip

/// Splice EXIF (APP1), IPTC (APP13), and a COM segment into a clean JPEG,
/// the way camera/phone files carry them.
fn jpeg_with_metadata() -> Vec<u8> {
    let clean = test_photo(600, 400);
    let exif_payload: Vec<u8> = {
        let mut v = b"Exif\0\0".to_vec();
        v.extend_from_slice(b"MM\0*\0\0\0\x08"); // TIFF header, IFD0 at offset 8
        v.extend_from_slice(&[0x00, 0x01]); // 1 IFD entry
        v.extend_from_slice(&[0x88, 0x27]); // tag: ISOSpeedRatings
        v.extend_from_slice(&[0x00, 0x03]); // type: SHORT
        v.extend_from_slice(&[0, 0, 0, 1]); // count: 1
        v.extend_from_slice(&[0x00, 0x64, 0, 0]); // value: 100
        v.extend_from_slice(&[0, 0, 0, 0]); // next IFD: none
        v.extend_from_slice(b" GPS CAMERA SERIAL DATA");
        v
    };
    let iptc: Vec<u8> = b"Photoshop 3.0\08BIM\x04\x04\0\0\0\0\0IPTC data"
        .iter()
        .copied()
        .collect();
    let comment = b"Edited by SomeEditor v1.2";

    let mut out = Vec::new();
    out.extend_from_slice(&clean[0..2]); // SOI
    for (marker, payload) in [(0xE1u8, &exif_payload), (0xEDu8, &iptc)] {
        out.extend_from_slice(&[0xFF, marker]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(payload);
    }
    out.extend_from_slice(&[0xFF, 0xFE]);
    out.extend_from_slice(&((comment.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(comment);
    out.extend_from_slice(&clean[2..]);
    out
}

#[test]
fn jpeg_strip_removes_metadata_losslessly() {
    let dirty = jpeg_with_metadata();
    // The inspector sees the planted metadata before stripping.
    let before = exif::jpeg_segments(&dirty);
    assert!(before.iter().any(|&(m, _)| m == 0xE1), "fixture must contain APP1");
    assert!(before.iter().any(|&(m, _)| m == 0xED), "fixture must contain APP13");
    assert!(before.iter().any(|&(m, _)| m == 0xFE), "fixture must contain COM");

    let stripped = strip_metadata(&dirty).unwrap();
    assert_eq!(detect_format(&stripped), Some(ImageFormat::Jpeg));
    let after = exif::jpeg_segments(&stripped);
    assert!(!after.iter().any(|&(m, _)| m == 0xE1), "APP1 must be gone");
    assert!(!after.iter().any(|&(m, _)| m == 0xED), "APP13 must be gone");
    assert!(!after.iter().any(|&(m, _)| m == 0xFE), "COM must be gone");
    // JFIF APP0 and the structural segments survive.
    assert!(after.iter().any(|&(m, _)| m == 0xE0), "APP0 (JFIF) must survive");
    assert!(after.iter().any(|&(m, _)| m == 0xDB), "DQT must survive");
    assert!(after.iter().any(|&(m, _)| m == 0xC4), "DHT must survive");

    // Lossless: decoding before/after yields identical pixels.
    let (a, b) = (decode(&dirty).unwrap(), decode(&stripped).unwrap());
    assert_eq!((a.width, a.height), (b.width, b.height));
    assert_eq!(a.data, b.data, "pixel data must be byte-identical after strip");

    // Idempotent.
    let twice = strip_metadata(&stripped).unwrap();
    assert_eq!(stripped, twice);
}

#[test]
fn png_strip_removes_exif_and_text_chunks() {
    // Build a PNG, then splice eXIf and tEXt chunks in before IDAT.
    let clean = test_photo(300, 200);
    let png = encode(&decode(&clean).unwrap(), ImageFormat::Png, 0).unwrap();

    let chunk = |out: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(ctype);
        out.extend_from_slice(data);
        let mut crc_input = Vec::new();
        crc_input.extend_from_slice(ctype);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    };
    // Metadata chunks follow IHDR in real files — the decoder rejects eXIf
    // before IHDR, so splice there.
    let ihdr_end = png
        .windows(4)
        .position(|w| w == *b"IHDR")
        .map(|i| i + 4 + 13 + 4)
        .expect("fixture PNG must have IHDR");
    let mut dirty = Vec::new();
    dirty.extend_from_slice(&png[0..ihdr_end]);
    chunk(&mut dirty, b"eXIf", b"MM\0*\0\0\0\x08\0\0");
    chunk(&mut dirty, b"tEXt", b"Software\0Secret Editor");
    dirty.extend_from_slice(&png[ihdr_end..]);

    assert!(exif::png_chunks(&dirty).contains(&*b"eXIf"));
    assert!(exif::png_chunks(&dirty).contains(&*b"tEXt"));

    let stripped = strip_metadata(&dirty).unwrap();
    let chunks = exif::png_chunks(&stripped);
    assert!(!chunks.contains(&*b"eXIf"), "eXIf must be gone");
    assert!(!chunks.contains(&*b"tEXt"), "tEXt must be gone");
    assert!(chunks.contains(&*b"IHDR"));
    assert!(chunks.contains(&*b"IDAT"));
    assert!(chunks.contains(&*b"IEND"));

    let (a, b) = (decode(&dirty).unwrap(), decode(&stripped).unwrap());
    assert_eq!(a.data, b.data, "PNG strip must be pixel-identical");
}

#[test]
fn strip_rejects_non_images() {
    assert!(strip_metadata(b"nope").is_err());
    assert!(strip_metadata(&[]).is_err());
}

/// Minimal CRC32 (PNG polynomial) for fixture building — the tpt-media-image
/// decoder verifies chunk CRCs on read, so the round-trip test only passes
/// if this is right.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *entry = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}
