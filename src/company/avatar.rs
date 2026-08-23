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

use crate::Result;
use crate::error::OpenCompanyError;

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
        assert!(err.contains("amber"), "the refusal must list what to pick: {err}");
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

    /// GIF is accepted deliberately (a moving face is more recognisable, not
    /// less); SVG is refused deliberately (a document that can carry script).
    #[test]
    fn image_types() {
        for ok in ["image/png", "image/jpeg", "image/webp", "image/gif", "IMAGE/GIF"] {
            assert!(is_supported_image(ok), "{ok}");
        }
        for no in ["image/svg+xml", "text/html", "application/pdf", ""] {
            assert!(!is_supported_image(no), "{no}");
        }
    }
}
