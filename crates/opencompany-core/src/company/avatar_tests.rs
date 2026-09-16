use super::*;

#[test]
fn accepts_every_shipped_flavour() {
    for flavour in TINY_FLAVOURS {
        let stored = normalize(&format!("tiny:{flavour}")).expect("a shipped flavour");
        assert_eq!(parse(&stored).unwrap(), AvatarRef::Tiny(flavour));
    }
}

#[test]
fn refuses_a_flavour_with_no_file() {
    // The whole point of validating: "puce" would render as a broken image
    // on every surface that draws a face, not just the one that set it.
    let err = parse("tiny:puce").unwrap_err().to_string();
    assert!(err.contains("puce"), "{err}");
    assert!(
        err.contains("amber"),
        "the refusal must list what to pick: {err}"
    );
}

#[test]
fn accepts_a_node_reference() {
    assert_eq!(
        parse("blob:01J8Z5Q9YQ0000000000000000").unwrap(),
        AvatarRef::Blob("01J8Z5Q9YQ0000000000000000")
    );
}

/// The security rule this module exists for: a URL is not an avatar. Each of
/// these is rendered into an `src=` on every surface that draws a face, so a
/// stored one is an instruction the console obeys for whoever wrote it.
#[test]
fn refuses_anything_that_is_not_one_of_the_two_forms() {
    for hostile in [
        "https://tracker.example/beacon.gif",
        "javascript:alert(1)",
        "data:image/gif;base64,R0lGOD",
        "/avatars/blob-amber.webp",
        "blob:../../etc/passwd",
        "blob:one two",
        "blob:",
        "",
        "amber",
    ] {
        let err = parse(hostile).unwrap_err().to_string();
        assert!(
            err.contains("A URL can't be stored as an avatar.") || err.contains("isn't one of"),
            "{hostile} was accepted or refused unhelpfully: {err}"
        );
    }
}

#[test]
fn refuses_an_unbounded_string() {
    assert!(parse(&format!("tiny:{}", "a".repeat(MAX_LEN))).is_err());
}

#[test]
fn trims_on_the_way_in() {
    assert_eq!(normalize("  tiny:teal \n").unwrap(), "tiny:teal");
}

#[test]
fn sniffs_the_four_accepted_formats() {
    assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
    assert_eq!(sniff_image(b"\xff\xd8\xff\xe0rest"), Some("image/jpeg"));
    assert_eq!(sniff_image(b"GIF89a...."), Some("image/gif"));
    assert_eq!(sniff_image(b"GIF87a...."), Some("image/gif"));
    assert_eq!(
        sniff_image(b"RIFF\x20\x00\x00\x00WEBPVP8 "),
        Some("image/webp")
    );
}

/// The point of sniffing rather than trusting the declared type: each of
/// these arrives labelled `image/png` by anyone who wants it to be.
#[test]
fn sniffing_refuses_what_only_claims_to_be_an_image() {
    for bytes in [
        &b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script/></svg>"[..],
        &b"<!doctype html><script>fetch('/')</script>"[..],
        &b"%PDF-1.7"[..],
        // A RIFF container that is not WebP — the near-miss the second half
        // of the WebP check exists for.
        &b"RIFF\x20\x00\x00\x00WAVEfmt "[..],
        &b""[..],
        &b"RIFF"[..],
    ] {
        assert_eq!(
            sniff_image(bytes),
            None,
            "{:?}",
            &bytes[..bytes.len().min(16)]
        );
    }
}

/// GIF is accepted deliberately (a moving face is more recognisable, not
/// less); SVG is refused deliberately (a document that can carry script).
#[test]
fn image_types() {
    for ok in [
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/gif",
        "IMAGE/GIF",
    ] {
        assert!(is_supported_image(ok), "{ok}");
    }
    for no in ["image/svg+xml", "text/html", "application/pdf", ""] {
        assert!(!is_supported_image(no), "{no}");
    }
}

// ——— decoded-size validation ——————————————————————————————

/// A PNG whose header announces the given size — the signature and IHDR
/// that carry width and height, plus the IHDR fields that follow them.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut v = PNG_SIGNATURE.to_vec();
    v.extend_from_slice(&13u32.to_be_bytes());
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0]);
    v
}

/// A GIF whose logical screen announces the given size.
fn gif(w: u16, h: u16) -> Vec<u8> {
    let mut v = GIF_SIGNATURE_89.to_vec();
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v
}

/// A GIF with the given logical screen and one Image Descriptor per entry
/// in `frames` — enough of the block stream for the frame walker to count
/// decoded pixels, with no color tables and empty raster data. The LZW
/// bytes are never decoded by the check being exercised, so empty sub-block
/// data is exactly what the parse needs.
fn gif_animated(logical: (u16, u16), frames: &[(u16, u16)]) -> Vec<u8> {
    let mut v = GIF_SIGNATURE_89.to_vec();
    v.extend_from_slice(&logical.0.to_le_bytes());
    v.extend_from_slice(&logical.1.to_le_bytes());
    // No global color table: flags 0, background 0, aspect 0.
    v.extend_from_slice(&[0x00, 0x00, 0x00]);
    for &(w, h) in frames {
        v.push(0x2C);
        v.extend_from_slice(&[0x00, 0x00]); // left
        v.extend_from_slice(&[0x00, 0x00]); // top
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.push(0x00); // no local color table
        v.push(0x02); // LZW minimum code size
        v.push(0x00); // zero-length raster data sub-block (the terminator)
    }
    v.push(0x3B); // trailer
    v
}

/// A minimal JPEG whose SOF0 announces the given size, preceded by an APP0
/// segment so the size is found by walking the marker list, not assumed at
/// an offset.
fn jpeg(w: u16, h: u16) -> Vec<u8> {
    let mut v = b"\xff\xd8".to_vec();
    // APP0 (JFIF), length 16, then a 14-byte payload.
    v.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    v.extend_from_slice(b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00");
    // SOF0, length 16: precision(1) + h(2) + w(2) + 3 components × 3.
    v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x10, 0x08]);
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&[0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
    v
}

/// A WebP whose VP8X canvas chunk announces the given size.
fn webp_vp8x(w: u32, h: u32) -> Vec<u8> {
    let (wm1, hm1) = (w - 1, h - 1);
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&22u32.to_le_bytes());
    v.extend_from_slice(b"WEBPVP8X");
    v.extend_from_slice(&10u32.to_le_bytes());
    v.extend_from_slice(&[
        0x00,
        0x00,
        0x00,
        0x00, //
        (wm1 & 0xFF) as u8,
        ((wm1 >> 8) & 0xFF) as u8,
        ((wm1 >> 16) & 0xFF) as u8,
        (hm1 & 0xFF) as u8,
        ((hm1 >> 8) & 0xFF) as u8,
        ((hm1 >> 16) & 0xFF) as u8,
    ]);
    v
}

/// A WebP whose VP8 (lossy) chunk announces the given size.
fn webp_vp8(w: u16, h: u16) -> Vec<u8> {
    let mut v = b"RIFF".to_vec();
    // 12 (container) + 8 (chunk header) + 10 (frame) = 30.
    v.extend_from_slice(&30u32.to_le_bytes());
    v.extend_from_slice(b"WEBPVP8 ");
    v.extend_from_slice(&10u32.to_le_bytes());
    // Frame tag + start code (RFC 6386), then width and height each as
    // their own little-endian 16-bit field, scale bits clear.
    v.extend_from_slice(&[0x9D, 0x01, 0x2A, 0x9D, 0x01, 0x2A]);
    v.push((w & 0xFF) as u8);
    v.push(((w >> 8) & 0x3F) as u8);
    v.push((h & 0xFF) as u8);
    v.push(((h >> 8) & 0x3F) as u8);
    v
}

/// A WebP whose VP8L (lossless) chunk announces the given size, with no
/// alpha (alpha_is_used = 0, version = 0).
///
/// Setting `alpha` toggles the alpha_is_used hint, so a regression test can
/// verify that the alpha and version bits are excluded from the height.
fn webp_vp8l(w: u32, h: u32, alpha: bool) -> Vec<u8> {
    let (wm1, hm1) = (w - 1, h - 1);
    let mut v = b"RIFF".to_vec();
    // 12 (container) + 8 (chunk header) + 5 (header) = 25.
    v.extend_from_slice(&25u32.to_le_bytes());
    v.extend_from_slice(b"WEBPVP8L");
    v.extend_from_slice(&5u32.to_le_bytes());
    // Signature (0x2F) + 14-bit (w−1) + 14-bit (h−1) + alpha + version(3).
    let payload = wm1 | (hm1 << 14) | ((alpha as u32) << 28);
    v.push(0x2F);
    v.extend_from_slice(&payload.to_le_bytes()[..4]);
    v
}

/// An animated WebP with the given VP8X canvas and one ANMF frame per entry
/// in `frames` — enough of the chunk stream for the frame walker to count
/// decoded pixels. Each ANMF carries its own `(width−1, height−1)` at the
/// fixed 24-bit offsets the walker reads, and a throwaway VP8 sub-chunk the
/// walker never looks inside.
fn webp_animated(canvas: (u32, u32), frames: &[(u32, u32)]) -> Vec<u8> {
    let (cw, ch) = canvas;
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&0u32.to_le_bytes()); // size, fixed below
    v.extend_from_slice(b"WEBP");
    // VP8X canvas with the animation flag (bit 1) set.
    v.extend_from_slice(b"VP8X");
    v.extend_from_slice(&10u32.to_le_bytes());
    v.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]);
    v.extend_from_slice(&(cw - 1).to_le_bytes()[..3]);
    v.extend_from_slice(&(ch - 1).to_le_bytes()[..3]);
    // ANIM chunk: background(3) + loop count(2).
    v.extend_from_slice(b"ANIM");
    v.extend_from_slice(&6u32.to_le_bytes());
    v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for &(fw, fh) in frames {
        let (fw1, fh1) = (fw - 1, fh - 1);
        // Frame header X(3) Y(3) W−1(3) H−1(3) duration(3) flags(1) — 16
        // bytes — then the frame's own VP8 sub-chunk (8 + 10).
        v.extend_from_slice(b"ANMF");
        v.extend_from_slice(&34u32.to_le_bytes());
        v.extend_from_slice(&[0x00, 0x00, 0x00]); // X
        v.extend_from_slice(&[0x00, 0x00, 0x00]); // Y
        v.extend_from_slice(&fw1.to_le_bytes()[..3]);
        v.extend_from_slice(&fh1.to_le_bytes()[..3]);
        v.extend_from_slice(&[0x0A, 0x00, 0x00]); // 10 ms
        v.push(0x00);
        v.extend_from_slice(b"VP8 ");
        v.extend_from_slice(&10u32.to_le_bytes());
        v.extend_from_slice(&[0x9D, 0x01, 0x2A, 0x9D, 0x01, 0x2A]);
        v.push((fw1 & 0xFF) as u8);
        v.push(((fw1 >> 8) & 0x3F) as u8);
        v.push((fh1 & 0xFF) as u8);
        v.push(((fh1 >> 8) & 0x3F) as u8);
    }
    let riff_size = (v.len() - 8) as u32;
    v[4..8].copy_from_slice(&riff_size.to_le_bytes());
    v
}

/// An APNG with the given IHDR canvas and one fcTL frame per entry in
/// `frames` — enough of the chunk stream for the frame walker to count
/// decoded pixels, with CRCs the walker ignores.
fn apng_animated(canvas: (u32, u32), frames: &[(u32, u32)]) -> Vec<u8> {
    let mut v = PNG_SIGNATURE.to_vec();
    // IHDR: the canvas.
    v.extend_from_slice(&13u32.to_be_bytes());
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&canvas.0.to_be_bytes());
    v.extend_from_slice(&canvas.1.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0]);
    v.extend_from_slice(&[0, 0, 0, 0]); // CRC (ignored)
    // acTL: frame count + plays.
    v.extend_from_slice(&8u32.to_be_bytes());
    v.extend_from_slice(b"acTL");
    v.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&[0, 0, 0, 0]); // CRC
    // One fcTL per frame: sequence, width, height, offsets, delays, ops.
    for (seq, &(fw, fh)) in frames.iter().enumerate() {
        v.extend_from_slice(&26u32.to_be_bytes());
        v.extend_from_slice(b"fcTL");
        v.extend_from_slice(&(seq as u32).to_be_bytes());
        v.extend_from_slice(&fw.to_be_bytes());
        v.extend_from_slice(&fh.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes()); // x offset
        v.extend_from_slice(&0u32.to_be_bytes()); // y offset
        v.extend_from_slice(&[0, 0, 0, 0]); // delay_num, delay_den
        v.extend_from_slice(&[0, 0]); // dispose_op, blend_op
        v.extend_from_slice(&[0, 0, 0, 0]); // CRC
    }
    // An IDAT so the file reads as a complete PNG; the walker never reaches it.
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(b"IDAT");
    v.extend_from_slice(&[0, 0, 0, 0]); // CRC
    v
}

#[test]
fn reads_the_size_each_format_announces() {
    assert_eq!(image_dimensions(&png(1, 1)).unwrap(), (1, 1));
    assert_eq!(image_dimensions(&png(192, 192)).unwrap(), (192, 192));
    assert_eq!(image_dimensions(&png(65535, 1)).unwrap(), (65535, 1));
    assert_eq!(image_dimensions(&gif(640, 480)).unwrap(), (640, 480));
    assert_eq!(image_dimensions(&jpeg(320, 240)).unwrap(), (320, 240));
    // A real mascot shape: a 192×192 VP8X canvas with an animated-style
    // VP8 frame following it (the canvas is what a decoder allocates).
    let mut extended = webp_vp8x(192, 192);
    extended.extend_from_slice(b"ANIM");
    extended.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(image_dimensions(&extended).unwrap(), (192, 192));
    assert_eq!(image_dimensions(&webp_vp8(192, 192)).unwrap(), (192, 192));
    assert_eq!(
        image_dimensions(&webp_vp8l(192, 192, false)).unwrap(),
        (192, 192)
    );
    assert_eq!(
        image_dimensions(&webp_vp8l(192, 192, true)).unwrap(),
        (192, 192)
    );
}

/// The VP8 height is a full 14 bits, not 10: the high six bits live in the
/// second size word's top bits (`data[9]`), and a parse that dropped them
/// measured a 4096×16383 frame as 4096×1023 — under the dimension cap, so
/// the decompression-bomb check let a 67-megapixel frame through.
#[test]
fn vp8_height_uses_all_fourteen_bits() {
    let (w, h) = (MAX_AVATAR_DIMENSION, 16383);
    let tall = webp_vp8(w as u16, h as u16);
    assert_eq!(image_dimensions(&tall).unwrap(), (w, h));
    assert!(
        check_image_dimensions(&tall).is_err(),
        "a 4096×16383 frame must be refused, not measured as 4096×1023"
    );
}

/// A real 1920×1080 lossy WebP: width and height each occupy their own
/// little-endian 16-bit field (RFC 6386 §9.1), the height bytes being
/// `0x38, 0x04`. A parse that packed the two together read that height as
/// 4320 and refused a perfectly ordinary landscape upload.
#[test]
fn vp8_height_is_its_own_two_byte_field() {
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&30u32.to_le_bytes());
    v.extend_from_slice(b"WEBPVP8 ");
    v.extend_from_slice(&10u32.to_le_bytes());
    v.extend_from_slice(&[0x9D, 0x01, 0x2A, 0x9D, 0x01, 0x2A]);
    // w = 1920 (0x0780), h = 1080 (0x0438), both scale bits clear.
    v.extend_from_slice(&[0x80, 0x07, 0x38, 0x04]);
    assert_eq!(image_dimensions(&v).unwrap(), (1920, 1080));
    assert!(
        check_image_dimensions(&v).is_ok(),
        "a 1920×1080 landscape WebP must be accepted"
    );
}

/// The VP8L (lossless) height is 14 bits; a parse that fails to mask out
/// the alpha_is_used and version bits reads a 192×192 lossless image with
/// alpha as 192×16576 and refuses the upload.
#[test]
fn vp8l_height_masks_alpha_and_version_bits() {
    // The flag is a non-normative hint; a real lossless file may have it
    // set, and version must be 0 for valid files.
    assert_eq!(
        image_dimensions(&webp_vp8l(192, 192, true)).unwrap(),
        (192, 192)
    );
    assert!(
        check_image_dimensions(&webp_vp8l(192, 192, true)).is_ok(),
        "a 192×192 lossless VP8L with alpha_is_used=1 must be accepted"
    );
    // A small VP8L with all version bits set (version = 7) must still
    // decode to the correct size — the spec requires version=0 but the
    // dimension parser must not read those bits as height.
    let bad_version = b"RIFF\x19\x00\x00\x00WEBPVP8L\x05\x00\x00\x00\x2F\x00\x00\x00\xE0";
    assert_eq!(
        image_dimensions(bad_version).unwrap(),
        (1, 1),
        "version bits must not corrupt the height"
    );
}

#[test]
fn size_check_accepts_a_reasonable_image() {
    for ok in [
        png(192, 192),
        gif(4096, 4096),
        jpeg(4032, 3024),
        webp_vp8x(4096, 4096),
    ] {
        check_image_dimensions(&ok).expect("a normal image must pass");
    }
}

/// The decompression bomb the caps exist for: a header claiming a huge
/// frame in a payload small enough to pass the 4 MiB ceiling.
#[test]
fn size_check_refuses_a_decompression_bomb() {
    for bomb in [
        png(65535, 65535),
        png(MAX_AVATAR_DIMENSION + 1, 1),
        gif(65535, 65535),
        jpeg(65535, 65535),
        webp_vp8x(65535, 65535),
        webp_vp8(65535, 65535),
    ] {
        let err = check_image_dimensions(&bomb).unwrap_err().to_string();
        assert!(
            err.contains("pixels") && err.contains("avatar has to fit"),
            "a bomb must be refused by name: {err}"
        );
    }
}

/// Both caps work together: an extreme aspect ratio whose edges each fit
/// within the dimension cap is still refused by total area.
#[test]
fn size_check_refuses_an_extreme_aspect_ratio() {
    let wide = png(MAX_AVATAR_DIMENSION * 2, MAX_AVATAR_DIMENSION / 2);
    assert!(
        check_image_dimensions(&wide).is_err(),
        "edges within the dimension cap must still respect the area cap"
    );
}

/// The frame walker sums every Image Descriptor's area, not just the
/// logical screen's.
#[test]
fn gif_animation_cost_counts_every_frame() {
    assert_eq!(
        gif_animation_cost(&gif_animated((100, 100), &[(100, 100), (50, 50)])).unwrap(),
        Some(12_500)
    );
    // Frames may be sub-rectangles of the screen; each one is still paid for.
    assert_eq!(
        gif_animation_cost(&gif_animated((4096, 4096), &[(128, 128)])).unwrap(),
        Some(16_384)
    );
    // Not a GIF, and a GIF with no Image Descriptor: nothing to count.
    assert_eq!(gif_animation_cost(PNG_SIGNATURE).unwrap(), None);
    assert_eq!(
        gif_animation_cost(&gif_animated((16, 16), &[])).unwrap(),
        None
    );
}

/// A global color table is skipped only when the descriptor's flag says one
/// is present — a walker that always skipped the table it expected would
/// misread the first block after a table-less header, and one that never
/// skipped it would read the table's bytes as block kinds.
#[test]
fn gif_animation_cost_skips_a_global_color_table_when_one_is_declared() {
    // Header + packed flags with the GCT flag (0x80) and size 0 (two
    // entries), then the 2 × 3-byte table, then one 16×16 frame.
    let mut v = GIF_SIGNATURE_89.to_vec();
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(&[0x80, 0x00, 0x00]);
    v.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    v.push(0x2C);
    v.extend_from_slice(&[0, 0, 0, 0]);
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.push(0x00); // no local color table
    v.push(0x02); // LZW min code size
    v.push(0x00); // empty raster data
    v.push(0x3B); // trailer
    assert_eq!(gif_animation_cost(&v).unwrap(), Some(256));
}

/// A GIF can hide a flood of full-canvas frames under the byte ceiling: the
/// logical screen fits the dimension caps and a single `4096²` frame would
/// too, but ten of them repaint ten times the decoded pixels every cycle.
#[test]
fn size_check_refuses_a_gif_that_animates_beyond_the_cost_cap() {
    let busy = gif_animated((4096, 4096), &[(4096, 4096); 10]);
    let err = check_image_dimensions(&busy).unwrap_err().to_string();
    assert!(
        err.contains("animates") && err.contains("per cycle"),
        "an animation far over the decoded-pixel cap must be refused by name: {err}"
    );

    // The same form, kept human: a small face with plenty of frames.
    let calm = gif_animated((128, 128), &[(128, 128); 60]);
    check_image_dimensions(&calm).expect("60 frames at 128×128 must pass");
}

/// The animated-WebP walker sums every ANMF rectangle, not the canvas size.
#[test]
fn webp_animation_cost_counts_every_anmf_frame() {
    assert_eq!(
        webp_animation_cost(&webp_animated((100, 100), &[(100, 100), (50, 50)])).unwrap(),
        Some(12_500)
    );
    // Frames may be sub-rectangles of the canvas; each one is still paid for.
    assert_eq!(
        webp_animation_cost(&webp_animated((4096, 4096), &[(128, 128)])).unwrap(),
        Some(16_384)
    );
    // Not a WebP, and a WebP with no ANMF chunks: nothing to count.
    assert_eq!(webp_animation_cost(PNG_SIGNATURE).unwrap(), None);
    assert_eq!(webp_animation_cost(&webp_vp8x(16, 16)).unwrap(), None);
}

/// The APNG walker pays for the default image (the canvas, frame 0 of the
/// cycle) plus every fcTL rectangle.
#[test]
fn apng_animation_cost_counts_the_canvas_and_every_fctl_frame() {
    assert_eq!(
        apng_animation_cost(&apng_animated((100, 100), &[(100, 100), (50, 50)])).unwrap(),
        Some(22_500)
    );
    // A still PNG carries no acTL, so there is nothing animated to count.
    assert_eq!(apng_animation_cost(&png(16, 16)).unwrap(), None);
}

/// A valid-looking animation whose stream is truncated after frames have
/// begun must not fall back to the still-image cap. Browsers decode the
/// frames that are present, so accepting this would let a frame flood past
/// `MAX_AVATAR_ANIMATED_PIXELS` by omitting only a trailer or later chunk.
#[test]
fn size_check_refuses_truncated_animations() {
    let mut gif_bytes = gif_animated((4096, 4096), &[(4096, 4096); 9]);
    gif_bytes.pop(); // remove the trailer
    let gif_err = check_image_dimensions(&gif_bytes).unwrap_err().to_string();
    assert!(gif_err.contains("truncated animation"), "GIF: {gif_err}");

    let mut webp = webp_animated((4096, 4096), &[(4096, 4096); 9]);
    webp.truncate(webp.len() - 1); // cut off the final ANMF payload
    let webp_err = check_image_dimensions(&webp).unwrap_err().to_string();
    assert!(webp_err.contains("truncated animation"), "WebP: {webp_err}");

    let mut apng_bytes = apng_animated((4096, 4096), &[(4096, 4096); 9]);
    apng_bytes.truncate(apng_bytes.len() - 1); // cut off the final chunk
    let apng_err = check_image_dimensions(&apng_bytes).unwrap_err().to_string();
    assert!(apng_err.contains("truncated animation"), "APNG: {apng_err}");

    // A header-only GIF has never reached a frame, so it remains the
    // accepted still-image case used by the normal-size test above.
    check_image_dimensions(&gif(4096, 4096)).expect("header-only GIF is still");
}
/// An animated WebP can hide a flood of full-canvas frames under the byte
/// ceiling exactly like a GIF can; the per-cycle walk must refuse it too.
#[test]
fn size_check_refuses_an_animated_webp_beyond_the_cost_cap() {
    let busy = webp_animated((4096, 4096), &[(4096, 4096); 10]);
    let err = check_image_dimensions(&busy).unwrap_err().to_string();
    assert!(
        err.contains("animates") && err.contains("per cycle"),
        "an animated WebP far over the decoded-pixel cap must be refused by name: {err}"
    );

    let calm = webp_animated((128, 128), &[(128, 128); 60]);
    check_image_dimensions(&calm).expect("60 frames at 128×128 must pass");
}

/// The same flood through an APNG: the default image plus every fcTL frame
/// is the per-cycle cost, and it is bounded like the other two formats.
#[test]
fn size_check_refuses_an_animated_apng_beyond_the_cost_cap() {
    let busy = apng_animated((4096, 4096), &[(4096, 4096); 8]);
    let err = check_image_dimensions(&busy).unwrap_err().to_string();
    assert!(
        err.contains("animates") && err.contains("per cycle"),
        "an animated APNG far over the decoded-pixel cap must be refused by name: {err}"
    );

    let calm = apng_animated((128, 128), &[(128, 128); 60]);
    check_image_dimensions(&calm).expect("60 frames at 128×128 must pass");
}

/// A payload too short to announce a size is not an image: a truncated
/// avatar would not decode anywhere either.
#[test]
fn size_check_refuses_a_truncated_payload() {
    for truncated in [
        PNG_SIGNATURE,
        &b"GIF89a"[..],
        &b"\xff\xd8\xff\xe0\x00\x10"[..],
        &b"RIFF\x16\x00\x00\x00WEBPVP8X"[..],
    ] {
        assert!(
            check_image_dimensions(truncated).is_err(),
            "{:?}",
            &truncated[..truncated.len().min(16)]
        );
    }
}

/// A SOF segment whose declared length is too short to hold the size bytes
/// used to slip past the segment-end check and then read past the buffer
/// when the fixed height/width indexes were applied. It must be refused,
/// not panic the request task.
#[test]
fn size_check_refuses_an_undersized_sof() {
    let undersized = b"\xff\xd8\xff\xc0\x00\x02";
    assert!(image_dimensions(undersized).is_none());
    assert!(check_image_dimensions(undersized).is_err());
}

// ——— resolve's referent rule ———————————————————————————————

fn binary_node(
    id: &str,
    parent: Option<&str>,
    mime: &str,
    origin: WorkspaceOrigin,
) -> WorkspaceNode {
    WorkspaceNode {
        name: format!("{id}.png"),
        id: id.to_string(),
        kind: NodeKind::File,
        parent_id: parent.map(str::to_string),
        updated_at_millis: 0,
        created_by: origin.clone(),
        updated_by: origin,
        mime: Some(mime.to_string()),
        size: None,
        sha256: None,
        adopted: false,
    }
}

fn folder_node(id: &str, name: &str) -> WorkspaceNode {
    WorkspaceNode {
        name: name.to_string(),
        id: id.to_string(),
        kind: NodeKind::Folder,
        parent_id: None,
        updated_at_millis: 0,
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: None,
        size: None,
        sha256: None,
        adopted: false,
    }
}

/// A scripted [`WorkspaceStore`] for exercising [`resolve`]'s referent
/// rule.
///
/// It answers exactly the reads `resolve` performs — the referent node and
/// its bytes, and the parent folder behind an immutability read — and
/// records the writes it performs, so a test can assert what a validated
/// copy became. Every other trait method is `unreachable!`: a test that
/// reaches one is resolving outside the branches it means to cover.
struct ScriptedStore {
    /// Referent id → (node, payload), served by both `read` and `read_bytes`.
    nodes: std::collections::HashMap<String, (WorkspaceNode, Vec<u8>)>,
    /// Folder id → node, for the parent read behind `avatar_node_is_immutable`.
    folders: std::collections::HashMap<String, WorkspaceNode>,
    /// The validated copies `create_binary` was asked to store.
    copies: std::sync::Mutex<Vec<(WorkspaceNode, Vec<u8>)>>,
}

impl ScriptedStore {
    fn new() -> Self {
        Self {
            nodes: std::collections::HashMap::new(),
            folders: std::collections::HashMap::new(),
            copies: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_node(mut self, node: WorkspaceNode, bytes: Vec<u8>) -> Self {
        self.nodes.insert(node.id.clone(), (node, bytes));
        self
    }

    fn with_folder(mut self, node: WorkspaceNode) -> Self {
        self.folders.insert(node.id.clone(), node);
        self
    }
}

#[async_trait::async_trait]
impl crate::ports::WorkspaceStore for ScriptedStore {
    async fn tree(
        &self,
        _company: &crate::ports::types::CompanyId,
    ) -> Result<Vec<WorkspaceNode>> {
        unreachable!("resolve does not list the tree")
    }
    async fn read(
        &self,
        _company: &crate::ports::types::CompanyId,
        id: &str,
    ) -> Result<Option<(WorkspaceNode, String)>> {
        if let Some((node, _)) = self.nodes.get(id) {
            return Ok(Some((node.clone(), String::new())));
        }
        Ok(self
            .folders
            .get(id)
            .map(|node| (node.clone(), String::new())))
    }

    async fn read_capped(
        &self,
        company: &crate::ports::types::CompanyId,
        id: &str,
        max_bytes: u64,
    ) -> Result<Option<(WorkspaceNode, String, u64)>> {
        crate::ports::workspace::read_capped_by_reading(self, company, id, max_bytes).await
    }
    async fn write_with_revision(
        &self,
        _company: &crate::ports::types::CompanyId,
        _id: &str,
        _content: &str,
        _author: WorkspaceOrigin,
        _expected_updated_at: Option<u64>,
    ) -> Result<WorkspaceNode> {
        unreachable!("resolve does not write prose")
    }
    async fn create(
        &self,
        _company: &crate::ports::types::CompanyId,
        _node: &WorkspaceNode,
        _content: Option<&str>,
    ) -> Result<()> {
        unreachable!("resolve does not create prose")
    }
    async fn adopt_or_create_folder(
        &self,
        _company: &crate::ports::types::CompanyId,
        _parent: Option<&str>,
        name: &str,
        _origin: WorkspaceOrigin,
    ) -> Result<crate::ports::workspace::FolderClaim> {
        Ok(crate::ports::workspace::FolderClaim::Created(folder_node(
            &format!("folder-{name}"),
            name,
        )))
    }
    async fn create_binary(
        &self,
        _company: &crate::ports::types::CompanyId,
        node: &WorkspaceNode,
        bytes: &[u8],
    ) -> Result<WorkspaceNode> {
        self.copies
            .lock()
            .expect("test double not poisoned")
            .push((node.clone(), bytes.to_vec()));
        Ok(node.clone())
    }
    async fn write_binary(
        &self,
        _company: &crate::ports::types::CompanyId,
        _id: &str,
        _bytes: &[u8],
        _mime: Option<&str>,
        _author: WorkspaceOrigin,
    ) -> Result<WorkspaceNode> {
        unreachable!("resolve does not rewrite bytes")
    }
    async fn read_bytes(
        &self,
        _company: &crate::ports::types::CompanyId,
        id: &str,
    ) -> Result<Option<(WorkspaceNode, crate::ports::workspace::BlobStream)>> {
        Ok(self.nodes.get(id).map(|(node, bytes)| {
            (
                node.clone(),
                crate::ports::workspace::one_chunk(bytes.clone()),
            )
        }))
    }
    async fn rename_move(
        &self,
        _company: &crate::ports::types::CompanyId,
        _id: &str,
        _name: Option<&str>,
        _parent: Option<Option<&str>>,
    ) -> Result<WorkspaceNode> {
        unreachable!("resolve does not move nodes")
    }
    async fn swap_files(
        &self,
        _company: &crate::ports::types::CompanyId,
        _expected_id: Option<&str>,
        _replacement_id: &str,
        _name: &str,
    ) -> Result<Option<WorkspaceNode>> {
        unreachable!("resolve does not swap files")
    }
    async fn delete(
        &self,
        _company: &crate::ports::types::CompanyId,
        _id: &str,
    ) -> Result<bool> {
        unreachable!("resolve does not delete")
    }
    async fn is_empty(&self, _company: &crate::ports::types::CompanyId) -> Result<bool> {
        unreachable!("resolve does not ask whether the tree is empty")
    }
}

#[tokio::test]
async fn resolve_refuses_a_missing_referent() {
    let store = ScriptedStore::new();
    let company = crate::ports::types::CompanyId::new("e2e");
    let err = resolve(&store, &company, "blob:nope")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("isn't here any more"), "{err}");
}

/// The store's own byte count is refused before any payload is buffered.
#[tokio::test]
async fn resolve_refuses_a_referent_the_store_counts_over_the_ceiling() {
    let company = crate::ports::types::CompanyId::new("e2e");
    let mut node = binary_node("big", None, "image/png", WorkspaceOrigin::Operator);
    node.size = Some(MAX_AVATAR_BYTES as u64 + 1);
    let store = ScriptedStore::new().with_node(node, png(16, 16));
    let err = resolve(&store, &company, "blob:big")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("can't be an avatar"), "{err}");
}

/// A store that leaves `size` unset is still bounded: the stream itself is
/// re-checked while it buffers.
#[tokio::test]
async fn resolve_refuses_a_referent_whose_stream_exceeds_the_byte_ceiling() {
    let company = crate::ports::types::CompanyId::new("e2e");
    let store = ScriptedStore::new().with_node(
        binary_node("huge", None, "image/png", WorkspaceOrigin::Operator),
        vec![0u8; MAX_AVATAR_BYTES + 1],
    );
    let err = resolve(&store, &company, "blob:huge")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("can't be an avatar"), "{err}");
}

/// A `blob:` can name any binary this host holds, and only the avatar route
/// sniffs before storing — so a node whose declared type disagrees with its
/// bytes would render as one face from this path and another from the Files
/// tab. A node with no declared type has nothing to agree with and is
/// refused too.
#[tokio::test]
async fn resolve_refuses_a_referent_whose_stored_type_disagrees() {
    let company = crate::ports::types::CompanyId::new("e2e");
    let store = ScriptedStore::new().with_node(
        binary_node("mislabeled", None, "image/png", WorkspaceOrigin::Operator),
        gif(16, 16),
    );
    let err = resolve(&store, &company, "blob:mislabeled")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("can't be an avatar"), "{err}");

    let mut bare = binary_node("bare", None, "image/png", WorkspaceOrigin::Operator);
    bare.mime = None;
    let store = ScriptedStore::new().with_node(bare, png(16, 16));
    let err = resolve(&store, &company, "blob:bare")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("can't be an avatar"), "{err}");
}

/// The upload route's own node — Operator origin under `avatars/` — is
/// already a validated copy that nothing rewrites, so resolve returns it as
/// the stored reference and mints nothing.
#[tokio::test]
async fn resolve_leaves_an_in_folders_node_untouched() {
    let company = crate::ports::types::CompanyId::new("e2e");
    let store = ScriptedStore::new()
        .with_folder(folder_node("folder-avatars", AVATARS_FOLDER))
        .with_node(
            binary_node(
                "avatar-1",
                Some("folder-avatars"),
                "image/png",
                WorkspaceOrigin::Operator,
            ),
            png(16, 16),
        );
    let stored = resolve(&store, &company, "blob:avatar-1")
        .await
        .expect("a stored reference");
    assert_eq!(stored, "blob:avatar-1");
    assert!(store.copies.lock().expect("not poisoned").is_empty());
}

/// The referent rule behind the provenance check: an artifact node that a
/// `PATCH …/workspace/{node}` moved beneath a folder named `avatars` still
/// carries its writer's origin, so it is not a face this host validated and
/// resolve must copy the bytes rather than store a reference to a node a
/// republish could rewrite.
#[tokio::test]
async fn resolve_copies_a_moved_artifact_node_before_storing() {
    let company = crate::ports::types::CompanyId::new("e2e");
    let store = ScriptedStore::new()
        .with_folder(folder_node("folder-avatars", AVATARS_FOLDER))
        .with_node(
            binary_node(
                "artifact-1",
                Some("folder-avatars"),
                "image/png",
                WorkspaceOrigin::Agent {
                    id: "image-bot".to_string(),
                },
            ),
            png(16, 16),
        );
    let stored = resolve(&store, &company, "blob:artifact-1")
        .await
        .expect("a validated copy");
    assert_ne!(
        stored, "blob:artifact-1",
        "a moved artifact node must not be stored by reference"
    );
    let copies = store.copies.lock().expect("not poisoned");
    assert_eq!(copies.len(), 1, "exactly one validated copy");
    let (node, bytes) = &copies[0];
    assert_eq!(node.parent_id.as_deref(), Some("folder-avatars"));
    assert_eq!(node.created_by, WorkspaceOrigin::Operator);
    assert_eq!(bytes, &png(16, 16));
}

/// A validated referent that lives outside the avatars folder is copied in
/// rather than stored by reference — the same immutable-copy rule as the
/// moved-artifact case, without the hostile parent.
#[tokio::test]
async fn resolve_copies_a_referent_that_lives_outside_the_avatars_folder() {
    let company = crate::ports::types::CompanyId::new("e2e");
    let store = ScriptedStore::new()
        .with_folder(folder_node("folder-files", "files"))
        .with_node(
            binary_node(
                "elsewhere",
                Some("folder-files"),
                "image/png",
                WorkspaceOrigin::Operator,
            ),
            png(16, 16),
        );
    let stored = resolve(&store, &company, "blob:elsewhere")
        .await
        .expect("a validated copy");
    assert_ne!(stored, "blob:elsewhere");
    let copies = store.copies.lock().expect("not poisoned");
    assert_eq!(copies.len(), 1);
    let (node, _) = &copies[0];
    assert_eq!(node.parent_id.as_deref(), Some("folder-avatars"));
    assert_eq!(node.created_by, WorkspaceOrigin::Operator);
}
