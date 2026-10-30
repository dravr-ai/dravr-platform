// ABOUTME: Fuel-product identity helpers: GTIN normalization, the brand/name/variant natural key, the source and photo hashes
// ABOUTME: Pure functions shared by the catalogue storage, the ingestion job and the review console (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use sha2::{Digest, Sha256};

/// A barcode as the GTIN-14 the catalogue stores, `None` if it is not one.
///
/// Spaces and hyphens are dropped, a UPC-A (12), EAN-13 or EAN-8 is
/// zero-padded to 14 digits, and the check digit is verified.
#[must_use]
pub fn normalize_gtin(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if !matches!(digits.len(), 8 | 12 | 13 | 14) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let padded = format!("{digits:0>14}");
    let values: Vec<u32> = padded.bytes().map(|b| u32::from(b - b'0')).collect();
    let (body, check) = values.split_at(13);
    // GTIN weights alternate 3,1,3,… from the leftmost of the 14 digits.
    let sum: u32 = body
        .iter()
        .enumerate()
        .map(|(i, d)| if i % 2 == 0 { d * 3 } else { *d })
        .sum();
    let expected = (10 - sum % 10) % 10;
    (check.first() == Some(&expected)).then_some(padded)
}

/// One identity part as the natural key compares it: the separator taken
/// out, then trimmed, whitespace collapsed and lowercased — in that order, so
/// "a | b" and "a|b" are one spelling.
fn key_part(part: &str) -> String {
    part.replace('|', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The key that makes a brand, name and variant one product: each part
/// normalized by Unicode lowercasing in Rust (never `lower()` in SQL, which
/// folds ASCII only on `SQLite`), joined by `|`.
#[must_use]
pub fn fuel_natural_key(brand: &str, name: &str, variant: &str) -> String {
    format!(
        "{}|{}|{}",
        key_part(brand),
        key_part(name),
        key_part(variant)
    )
}

/// Lowercase hex of a SHA-256 digest.
fn hex(digest: &[u8]) -> String {
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// SHA-256 of one photo's bytes, lowercase hex, as a proposal stores it.
#[must_use]
pub fn image_sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The dedupe hash of a proposal's source: SHA-256, lowercase hex.
///
/// Taken over the page URL, then every photo sent, in order, each prefixed
/// with its length so two photo sets never collide by concatenation. The same
/// photos in another order, or under another page, are another source.
#[must_use]
pub fn proposal_source_hash(source_url: Option<&str>, images: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_url.unwrap_or_default().as_bytes());
    hasher.update([0_u8]);
    for image in images {
        hasher.update((image.len() as u64).to_le_bytes());
        hasher.update(image);
    }
    hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtin_normalizes_upc_ean13_and_ean8() {
        assert_eq!(
            normalize_gtin("036000291452").as_deref(),
            Some("00036000291452")
        );
        assert_eq!(
            normalize_gtin("4006381333931").as_deref(),
            Some("04006381333931")
        );
        assert_eq!(
            normalize_gtin("96385074").as_deref(),
            Some("00000096385074")
        );
        assert_eq!(
            normalize_gtin("4006-381 333931").as_deref(),
            Some("04006381333931")
        );
        assert_eq!(
            normalize_gtin("00036000291452").as_deref(),
            Some("00036000291452")
        );
    }

    #[test]
    fn gtin_rejects_bad_check_digits_lengths_and_letters() {
        assert_eq!(normalize_gtin("036000291453"), None);
        assert_eq!(normalize_gtin("4006381333932"), None);
        assert_eq!(normalize_gtin("96385075"), None);
        assert_eq!(normalize_gtin("12345"), None);
        assert_eq!(normalize_gtin("40063813339a1"), None);
        assert_eq!(normalize_gtin(""), None);
    }

    #[test]
    fn natural_key_folds_case_and_whitespace_unicode_included() {
        assert_eq!(
            fuel_natural_key("  NÄAK ", "Ultra   Energy", ""),
            fuel_natural_key("näak", "ultra energy", "  ")
        );
        assert_ne!(
            fuel_natural_key("a|b", "c", ""),
            fuel_natural_key("a", "b|c", "")
        );
        // A separator in a part is whitespace, however it is spaced.
        assert_eq!(
            fuel_natural_key("Brand", "Gel | Citrus", ""),
            fuel_natural_key("brand", "gel|citrus", "")
        );
    }

    #[test]
    fn source_hash_is_stable_hex_and_url_sensitive() {
        let a = proposal_source_hash(Some("https://x/1"), &[b"img"]);
        assert_eq!(a.len(), 64);
        assert_eq!(a, proposal_source_hash(Some("https://x/1"), &[b"img"]));
        assert_ne!(a, proposal_source_hash(Some("https://x/2"), &[b"img"]));
    }

    #[test]
    fn source_hash_covers_every_photo_in_order() {
        let url = Some("https://naak.com/products/gel");
        let set: [&[u8]; 2] = [b"front", b"panel"];
        assert_ne!(
            proposal_source_hash(url, &set),
            proposal_source_hash(url, &[b"panel", b"front"])
        );
        assert_ne!(
            proposal_source_hash(url, &set),
            proposal_source_hash(url, &[b"panel"])
        );
        // Length prefixes: moving a byte between photos is a different set.
        assert_ne!(
            proposal_source_hash(url, &[b"ab", b"c"]),
            proposal_source_hash(url, &[b"a", b"bc"])
        );
    }

    #[test]
    fn image_sha256_is_the_plain_digest() {
        assert_eq!(
            image_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
