//! Avatar references: which face a teammate or a person wears.
//!
//! Every teammate and every person already *has* a face — the console hashes
//! their stable id into one of the mascots shipped in `frontend/public/avatars/`
//! and draws that, which is why a company with nobody's avatar set still reads
//! as a roster of individuals rather than a column of grey squares. This module
//! is the other half: what is stored when somebody **chooses** a face instead.
//!
//! ## The grammar
//!
//! An avatar reference is one short string in exactly one of two forms:
//!
//! | Form | Means |
//! |---|---|
//! | `tiny:<flavour>` | one of the [shipped mascots](TINY_FLAVOURS) — a flavour of tiny |
//! | `blob:<nodeId>` | a custom image the operator uploaded, held as a binary workspace node |
//!
//! Absent (`None`) is a third state and the default: *nobody has chosen*, so the
//! console keeps hashing. It is deliberately distinct from either stored form,
//! because "reset to the default face" has to be expressible and neither
//! `tiny:` nor an empty string can express it.
//!
//! ## Why the grammar is closed
//!
//! The obvious shape is to store a URL and be done. That is exactly what this
//! refuses, and the reason is that the string ends up in an `src=` attribute on
//! every console surface that draws a face — chat gutters, facepiles, the org
//! chart, the members pane. A stored URL is therefore an instruction the console
//! obeys on behalf of whoever wrote it: `javascript:` is script injection,
//! `http://tracker.example/x.gif` is a beacon that fires for every viewer and
//! reports who looked at the roster and when, and either survives in the record
//! long after the person who set it lost their account.
//!
//! Both stored forms name something *this host already holds*, so rendering one
//! reaches nothing the viewer's session did not already reach.
//!
//! ## Animation
//!
//! GIFs are first-class: an avatar is a small square that a person picked to be
//! recognisable, and a moving one is more recognisable, not less. Nothing here
//! transcodes, so an animated GIF or WebP is stored and served as the bytes that
//! were uploaded and animates wherever the console draws it. See
//! [`is_supported_image`] for the accepted types, and the upload route for the
//! size ceiling.

use futures::StreamExt;

use crate::Result;
use crate::error::OpenCompanyError;

/// The leading bytes that announce each accepted format.
///
/// Hoisted so `sniff_image` and [`image_dimensions`] read the same signatures
/// and cannot drift apart — one checks *that* the bytes name a format, the
/// other *what size* the format the bytes name claims.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_SIGNATURE: &[u8] = b"\xff\xd8\xff";
const GIF_SIGNATURE_87: &[u8] = b"GIF87a";
const GIF_SIGNATURE_89: &[u8] = b"GIF89a";

/// The mascots shipped with the console, one file per colourway.
///
/// **Must stay in step with `frontend/public/avatars/blob-<flavour>.webp` and
/// with `TINY_FLAVOURS` in `frontend/src/lib/avatar.ts`.** A flavour accepted
/// here that has no file renders as a broken image on every surface at once,
/// which is why the host validates the name rather than storing whatever it is
/// handed.
pub const TINY_FLAVOURS: [&str; 11] = [
    "amber", "blue", "clay", "cloud", "ember", "graphite", "green", "indigo", "rose", "teal",
    "violet",
];

/// The longest an avatar reference may be.
///
/// Both forms are a short prefix plus an identifier the host itself minted, so
/// this is far above anything legitimate; it exists so an unbounded string
/// cannot be pushed into a record through a field nobody thought to bound.
const MAX_LEN: usize = 128;

/// A parsed avatar reference — where the face actually comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarRef<'a> {
    /// One of the [shipped mascots](TINY_FLAVOURS), by flavour name.
    Tiny(&'a str),
    /// A custom image, by the id of the binary workspace node holding its bytes.
    Blob(&'a str),
}

/// The image types an uploaded avatar may be.
///
/// GIF is on the list on purpose — see the module docs. SVG is **not**: an SVG
/// is a document that can carry script and fetch remote resources, so accepting
/// one would reintroduce, inside a file, precisely what refusing arbitrary URLs
/// keeps out.
pub fn is_supported_image(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

/// The largest an uploaded avatar may be.
///
/// Four mebibytes is generous for a square somebody will see at 32px and mean
/// for a phone photo, which is the trade being made: it has to fit an animated
/// GIF with enough frames to be worth animating, and it must not let the roster
/// become a place to park a video. The console shrinks a still image before it
/// uploads; an animated one cannot be shrunk without transcoding it, so this
/// ceiling is what an animation is actually held to.
pub const MAX_AVATAR_BYTES: usize = 4 * 1024 * 1024;

/// The largest edge an avatar image may have, in pixels.
///
/// Four thousand is far above anything a face is drawn at — the console shows
/// these at tens of pixels — and still small enough that no decoder's per-image
/// allocation is worth a second thought. Together with [`MAX_AVATAR_PIXELS`]
/// this is what turns a decompression bomb (a header claiming 65535×65535 in a
/// body of a few hundred compressed bytes) into a `400` on the upload instead
/// of a gigabyte allocation for every operator who views the roster.
pub const MAX_AVATAR_DIMENSION: u32 = 4096;

/// The largest total area an avatar image may decode to, in pixels.
///
/// A square with both edges at [`MAX_AVATAR_DIMENSION`] is the largest shape
/// the two caps accept; this is what refuses an extreme aspect ratio whose
/// edges each happen to fit. `4096²` is 16 megapixels — an RGBA buffer of
/// 64 MiB, the worst case any surface ever decodes for one face.
pub const MAX_AVATAR_PIXELS: u64 = 4096 * 4096;

/// The media type these bytes actually are, read from their signature.
///
/// **Not** the type the upload declared. A declared type is a claim by whoever
/// is uploading, and an avatar is stored once and then served back to every
/// member of the company for as long as the teammate exists — so the type it is
/// served under has to be a fact about the bytes rather than a claim about them.
/// The four accepted formats all begin with an unambiguous signature, so this
/// costs a dozen bytes to answer honestly.
///
/// `None` means "not one of the four", which the upload route refuses. In
/// particular an SVG, an HTML document and a PDF all land here as `None`
/// whatever they were labelled as.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(PNG_SIGNATURE) {
        return Some("image/png");
    }
    if bytes.starts_with(JPEG_SIGNATURE) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(GIF_SIGNATURE_87) || bytes.starts_with(GIF_SIGNATURE_89) {
        return Some("image/gif");
    }
    // RIFF....WEBP — the four size bytes in between are part of the container,
    // so both ends of the signature have to be checked for this to mean WebP
    // rather than "some RIFF file", of which .wav is the commonest.
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// The decoded dimensions of a supported image, read from its own header.
///
/// [`sniff_image`] answers "do the leading bytes *name* one of the four
/// formats"; this answers "and what size does the format those bytes name
/// claim". The gap between the two is the decompression bomb: a header can
/// promise 65535×65535 in a payload small enough to pass the avatar ceiling,
/// and a decoder that trusts the promise hands the allocation to whoever
/// views it. Reading the announced size before the bytes are stored turns that
/// from a per-viewer allocation into a `400` on the upload.
///
/// A header parse, not a decode — decoding to learn the size would be the
/// allocation the check exists to prevent.
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(PNG_SIGNATURE) {
        // Signature (8) + IHDR length (4) + "IHDR" (4), then big-endian width
        // and height at a fixed offset.
        let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if bytes.starts_with(GIF_SIGNATURE_87) || bytes.starts_with(GIF_SIGNATURE_89) {
        // Logical screen width and height, little-endian, right after the
        // signature.
        let w = u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?) as u32;
        let h = u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?) as u32;
        return Some((w, h));
    }
    if bytes.starts_with(JPEG_SIGNATURE) {
        return jpeg_dimensions(bytes);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return webp_dimensions(bytes);
    }
    None
}

/// The height and width a JPEG announces, from its SOF segment.
///
/// The SOF marker carries the frame size, and it may sit after any number of
/// APPn/COM/DQT/DHT/DRI segments — there is no fixed offset — so the marker
/// list has to be walked. Each segment is `FF` + marker + 2-byte length; the
/// length covers itself and the payload but not the marker.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // Start after the SOI (`FF D8`), which `sniff_image` has already required.
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        // `FF FF` is a fill byte; the second FF begins the real marker.
        if bytes[i + 1] == 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // Standalone markers carry no length field: SOI, EOI, RSTn, TEM.
        if marker == 0xD8 || marker == 0xD9 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        // A SOF segment is precision(1) + height(2) + width(2), then the
        // per-component bytes. The SOF markers are C0–CF except the ones that
        // are not SOF: DHT (C4), JPG (C8), DAC (CC), DNL (DC), DRI (DD).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC | 0xDC | 0xDD)
        {
            // `len` covers the length field and the payload but not the marker,
            // so the segment ends at `i + 2 + len`. A real SOF always carries
            // at least one component, so `len >= 10` and the size bytes below
            // are guaranteed to sit inside it.
            if i + 2 + len > bytes.len() {
                return None;
            }
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// The width and height a WebP announces, from its first image chunk.
///
/// After the 12-byte RIFF/WEBP container, chunks are FourCC (4) + size (4) +
/// payload, padded to an even byte count. The canvas size lives in the first
/// image chunk — VP8X when the file is extended (alpha, animation), else VP8
/// (lossy) or VP8L (lossless).
fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let fourcc = bytes.get(i..i + 4)?;
        let size = u32::from_le_bytes(bytes.get(i + 4..i + 8)?.try_into().ok()?) as usize;
        let data = bytes.get(i + 8..i + 8 + size)?;
        match fourcc {
            b"VP8 " => {
                // Lossy: frame tag (3) + start code (3), then 14-bit width and
                // height (RFC 6386 §9.1).
                if data.len() < 9 {
                    return None;
                }
                let w = (data[6] as u32) | (((data[7] & 0x3F) as u32) << 8);
                let h = (((data[7] & 0xC0) as u32) >> 6) | ((data[8] as u32) << 2);
                return Some((w, h));
            }
            b"VP8L" => {
                // Lossless: `0x2F` signature, then 14-bit (width−1) and 14-bit
                // (height−1).
                if data.len() < 5 || data[0] != 0x2F {
                    return None;
                }
                let w = 1 + ((data[1] as u32) | (((data[2] & 0x3F) as u32) << 8));
                let h = 1
                    + ((((data[2] & 0xC0) as u32) >> 6)
                        | ((data[3] as u32) << 2)
                        | ((data[4] as u32) << 10));
                return Some((w, h));
            }
            b"VP8X" => {
                // Extended: flags (1) + reserved (3) + 24-bit (width−1) and
                // (height−1).
                if data.len() < 10 {
                    return None;
                }
                let w = 1 + ((data[4] as u32) | ((data[5] as u32) << 8) | ((data[6] as u32) << 16));
                let h = 1 + ((data[7] as u32) | ((data[8] as u32) << 8) | ((data[9] as u32) << 16));
                return Some((w, h));
            }
            // ALPH, ANIM, ANMF, ICCP, EXIF, XMP and anything unknown are
            // skipped — the first image chunk has already been read by the
            // time a file reaches them.
            _ => {}
        }
        i += 8 + size + (size & 1);
    }
    None
}

/// Refuses an image whose decoded size is a decompression bomb.
///
/// [`sniff_image`] proves the leading bytes *name* one of the four formats;
/// this proves the image they name is not pathological. An avatar is rendered
/// at a handful of pixels, so an edge over [`MAX_AVATAR_DIMENSION`] or an area
/// over [`MAX_AVATAR_PIXELS`] has nothing legitimate behind it — it is a
/// header that would make every decoder that touches it allocate a buffer
/// nobody needs. A payload too short to announce a size is refused as not an
/// image: a truncated avatar would not decode anywhere either.
pub fn check_image_dimensions(bytes: &[u8]) -> Result<()> {
    let Some((w, h)) = image_dimensions(bytes) else {
        return Err(not_an_image());
    };
    if w > MAX_AVATAR_DIMENSION
        || h > MAX_AVATAR_DIMENSION
        || (w as u64) * (h as u64) > MAX_AVATAR_PIXELS
    {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "that image is {w}×{h} pixels — an avatar has to fit within \
             {MAX_AVATAR_DIMENSION}×{MAX_AVATAR_DIMENSION}."
        )));
    }
    Ok(())
}

/// Parses a stored or submitted avatar reference.
///
/// Returns [`OpenCompanyError::InvalidRequest`] naming both accepted forms,
/// because the commonest way to get this wrong is to send a URL and the error
/// has to say what to send instead.
pub fn parse(value: &str) -> Result<AvatarRef<'_>> {
    let value = value.trim();
    if value.len() > MAX_LEN {
        return Err(refusal());
    }
    if let Some(flavour) = value.strip_prefix("tiny:") {
        return if TINY_FLAVOURS.contains(&flavour) {
            Ok(AvatarRef::Tiny(flavour))
        } else {
            Err(OpenCompanyError::InvalidRequest(format!(
                "\"{flavour}\" isn't one of the tiny avatars. Pick one of: {}.",
                TINY_FLAVOURS.join(", ")
            )))
        };
    }
    if let Some(node) = value.strip_prefix("blob:") {
        return if is_node_id(node) {
            Ok(AvatarRef::Blob(node))
        } else {
            Err(refusal())
        };
    }
    Err(refusal())
}

/// Validates a submitted reference and returns the form to store.
///
/// Trimming here rather than at each call site is what keeps a copy-pasted
/// value with a trailing space from being stored as a reference that parses
/// nowhere.
pub fn normalize(value: &str) -> Result<String> {
    parse(value)?;
    Ok(value.trim().to_string())
}

/// Whether `node` could be a workspace node id.
///
/// Node ids are ULIDs, but this deliberately checks the *character set* rather
/// than the format: the id is interpolated into a route path by the console, so
/// what matters is that it cannot carry a separator or an escape. Whether it
/// names a node that exists is the read's answer, not this function's — a
/// deleted avatar node is a 404 on one image, which the console draws as the
/// hashed default.
fn is_node_id(node: &str) -> bool {
    !node.is_empty()
        && node.len() <= 64
        && node
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn refusal() -> OpenCompanyError {
    OpenCompanyError::InvalidRequest(format!(
        "an avatar must be \"tiny:<flavour>\" (one of: {}) or \"blob:<nodeId>\" for an uploaded \
         image. A URL can't be stored as an avatar.",
        TINY_FLAVOURS.join(", ")
    ))
}

/// Validates a submitted reference **and what it points at**, returning the form
/// to store.
///
/// [`normalize`] answers "is this a well-formed reference"; this answers "is
/// there an image here". The difference matters because a `blob:` reference is
/// just a node id, and any member can type one: pointed at a 60 MB PDF it makes
/// every surface that draws a face try to decode a PDF as an image, on every
/// page load, for everyone. Checking the referent turns that from a thing the
/// roster does into a `400` on the request that asked for it.
///
/// A `tiny:` reference needs no lookup — the file is shipped with the console —
/// so the store is only touched for the form that names something mutable.
pub async fn resolve(
    workspace: &dyn crate::ports::WorkspaceStore,
    company: &crate::ports::types::CompanyId,
    value: &str,
) -> Result<String> {
    let stored = normalize(value)?;
    let AvatarRef::Blob(node_id) = parse(&stored)? else {
        return Ok(stored);
    };
    let Some((node, stream)) = workspace.read_bytes(company, node_id).await? else {
        return Err(OpenCompanyError::InvalidRequest(
            "that image isn't here any more. Upload it again.".to_string(),
        ));
    };
    // The store's own byte count, when it has one, refused before any of the
    // payload is buffered. The stream below re-checks with the bytes
    // themselves, so a store that leaves `size` unset is still bounded.
    if let Some(size) = node.size
        && size > MAX_AVATAR_BYTES as u64
    {
        return Err(not_an_image());
    }
    // The bytes themselves are the only claim worth trusting. A `blob:`
    // reference can name any binary this host holds, and the type a generic
    // workspace upload declared is a claim by whoever uploaded it — a member
    // can reach the 4 MiB avatar ceiling with `image/png` on arbitrary bytes.
    // Sniffing here closes the gap between the avatar route (which sniffs
    // before storing) and a reference typed by hand.
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = stream;
    while let Some(chunk) = stream.next().await.transpose()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_AVATAR_BYTES {
            return Err(not_an_image());
        }
        bytes.extend_from_slice(&chunk);
    }
    match sniff_image(&bytes) {
        Some(sniffed) => {
            // The bytes are a supported format; now make sure they are not a
            // decompression bomb. A hand-typed `blob:` can name any binary this
            // host holds, and one that claims 65535×65535 in a few compressed
            // bytes would make every surface that draws a face allocate it.
            check_image_dimensions(&bytes)?;
            // The bytes and the stored type have to agree, because the type a
            // node was stored under is a claim by whoever uploaded it: the
            // generic workspace route keeps the declared `Content-Type`, and
            // only the avatar route sniffs before storing. A `blob:` whose
            // stored `image/png` is really a GIF would render as a PNG from
            // this path and as a GIF from the Files tab — the same bytes, two
            // faces. A node with no declared type has nothing to agree with,
            // so it is refused too; every path that stores an avatar records
            // the sniffed type.
            let essence = node
                .mime
                .as_deref()
                .and_then(|m| m.split(';').next())
                .map(str::trim)
                .map(str::to_ascii_lowercase);
            if essence.as_deref() == Some(sniffed) {
                Ok(stored)
            } else {
                Err(not_an_image())
            }
        }
        None => Err(not_an_image()),
    }
}

/// The refusal a `blob:` reference gets when its bytes are not a supported
/// image or are over the avatar ceiling. One sentence for both: from the
/// caller's side these are one failure — "that isn't an avatar" — and two
/// different sentences for it would read as two different problems.
fn not_an_image() -> OpenCompanyError {
    OpenCompanyError::InvalidRequest(
        "that file isn't a PNG, JPEG, GIF or WebP image, so it can't be an avatar.".to_string(),
    )
}

#[cfg(test)]
mod test {
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
}
