// ABOUTME: Fuel-product identity helpers: GTIN normalization, the brand/name/variant natural key, the source, read and photo hashes
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

/// Hash `field` into `hasher` prefixed with its length, so two fields never
/// run together.
fn update_field(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_le_bytes());
    hasher.update(field);
}

/// What a label read sends besides its page and photos: everything that
/// can change the answer, so each is part of [`fuel_read_hash`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuelReadRequest<'req> {
    /// The `fuel_panel_extraction` prompt text sent.
    pub prompt: &'req str,
    /// The user message the photos travel on, as sent: rewording it can
    /// change the answer as much as rewording the prompt.
    pub user_message: &'req str,
    /// The configured model, as `provider=model`.
    pub model: &'req str,
    /// The request's output cap: an answer cut off at a lower one may parse
    /// at a higher one.
    pub max_tokens: u32,
    /// The request's sampling temperature.
    pub temperature: f32,
}

/// The hash of one label read the ingestion job pays for.
///
/// SHA-256, lowercase hex, over the page URL, the prompt text sent, the user
/// message the photos travel on, the model it was sent to, the output cap and
/// temperature, and every photo sent, in order, each field length-prefixed. A
/// read with this hash on file is not paid for again; a new photo, a reworded
/// `fuel_panel_extraction` prompt or user message, another configured model or
/// a raised output cap is a new read. The
/// proposal the read files carries the same hash.
#[must_use]
pub fn fuel_read_hash(source_url: &str, images: &[&[u8]], request: FuelReadRequest<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"fuel-ingest-read\0");
    update_field(&mut hasher, source_url.as_bytes());
    update_field(&mut hasher, request.prompt.as_bytes());
    update_field(&mut hasher, request.user_message.as_bytes());
    update_field(&mut hasher, request.model.as_bytes());
    hasher.update(request.max_tokens.to_le_bytes());
    hasher.update(request.temperature.to_bits().to_le_bytes());
    hasher.update((images.len() as u64).to_le_bytes());
    for image in images {
        update_field(&mut hasher, image);
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
    fn a_read_hash_changes_with_the_photos_the_prompt_the_message_the_model_and_the_cap() {
        let url = "https://naak.com/products/gel";
        let set: [&[u8]; 2] = [b"front", b"panel"];
        let opus = FuelReadRequest {
            prompt: "prompt v1",
            user_message: "Transcribe the label shown in these 2 photo(s) of one package.",
            model: "anthropic=claude-opus-5-5",
            max_tokens: 4_000,
            temperature: 0.0,
        };
        let base = fuel_read_hash(url, &set, opus);
        assert_eq!(base.len(), 64);
        assert_eq!(base, fuel_read_hash(url, &set, opus));
        for other in [
            fuel_read_hash(url, &[b"panel", b"front"], opus),
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    prompt: "prompt v2",
                    ..opus
                },
            ),
            // The same photos sent on a reworded user message.
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    user_message: "Transcribe the nutrition panel in these 2 photo(s).",
                    ..opus
                },
            ),
            // Text moved between the prompt and the user message.
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    prompt: "prompt v1T",
                    user_message: "ranscribe the label shown in these 2 photo(s) of one package.",
                    ..opus
                },
            ),
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    model: "anthropic=claude-haiku-5-5",
                    ..opus
                },
            ),
            // A raised output cap reads again: an answer cut off mid-object
            // at 2,000 tokens (`parse_failed`) may parse at 4,000.
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    max_tokens: 2_000,
                    ..opus
                },
            ),
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    temperature: 0.2,
                    ..opus
                },
            ),
            fuel_read_hash("https://naak.com/products/bar", &set, opus),
            // Length prefixes: text moved between the prompt and the model
            // is another read.
            fuel_read_hash(
                url,
                &set,
                FuelReadRequest {
                    prompt: "prompt v",
                    model: "1anthropic=claude-opus-5-5",
                    ..opus
                },
            ),
        ] {
            assert_ne!(base, other);
        }
        assert_ne!(base, proposal_source_hash(Some(url), &set));
    }

    #[test]
    fn image_sha256_is_the_plain_digest() {
        assert_eq!(
            image_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
