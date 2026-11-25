// ABOUTME: A consent grant receipt signed as a compact JWS (EdDSA) by Dravr's checkpoint key, kid named by did:web
// ABOUTME: The protected header is fixed in order; the signing input is a JWS's, domain-separated from the key's other messages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Grant receipts as JWS (carnet#1544, owl §8.1)
//!
//! A receipt's payload (`pierre_core::models::GrantReceipt`) is signed as an
//! RFC 7515 compact JWS by the `record_log` checkpoint signer
//! ([`crate::barrage_keys::checkpoint_signer`]), the custodian key a verifier
//! reads from `/.well-known/did.json`: the protected header is, in this
//! order, `{"alg":"EdDSA","typ":"health-grant-receipt+jws","kid":
//! "did:web:<host>#checkpoint","iss":"<issuer URL>"}`. No `hp_dcert` and no
//! `crit`: the key is Dravr's, not a device's.
//!
//! ## Domain separation
//!
//! The checkpoint key signs three message families. A JWS must sign exactly
//! `BASE64URL(header) '.' BASE64URL(payload)`, so no text prefix can be put
//! in front of it; separation is by form instead. That input starts with
//! `eyJ` (the base64url of `{"`) and holds no newline, so it is never a
//! checkpoint note (first line
//! [`CHECKPOINT_ORIGIN`](crate::barrage_keys::CHECKPOINT_ORIGIN)) nor a tool-definition
//! message (first line `dravr.ai/tool-def`, [`TOOL_DEF_SIG_CONTEXT`](crate::barrage_keys::TOOL_DEF_SIG_CONTEXT)), and
//! neither of those is a JWS input. The explicit `typ` separates receipts
//! from other JWSs (RFC 8725 §3.11).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use dravr_barrage::keyed::Signer;
use pierre_core::errors::{AppError, AppResult};
use serde::Serialize;

use crate::barrage_keys::checkpoint_key_id;

/// The JOSE algorithm of every receipt: Ed25519.
const GRANT_RECEIPT_JWS_ALG: &str = "EdDSA";

/// The `typ` every receipt's JWS carries (RFC 8725 §3.11).
const GRANT_RECEIPT_JWS_TYPE: &str = "health-grant-receipt+jws";

/// The protected header, in its fixed order.
#[derive(Serialize)]
struct ProtectedHeader<'header> {
    alg: &'static str,
    typ: &'static str,
    kid: &'header str,
    iss: &'header str,
}

/// The JWS signing input for `payload`, signed under `issuer_url`:
/// `BASE64URL(header) '.' BASE64URL(payload)`.
///
/// # Errors
/// An internal error when `issuer_url` has no host, so the key has no
/// `did:web` name.
fn signing_input(issuer_url: &str, payload: &str) -> AppResult<String> {
    let kid = checkpoint_key_id(issuer_url).ok_or_else(|| {
        AppError::internal(format!(
            "grant receipt: the issuer URL `{issuer_url}` has no host to name the key under"
        ))
    })?;
    let header = serde_json::to_string(&ProtectedHeader {
        alg: GRANT_RECEIPT_JWS_ALG,
        typ: GRANT_RECEIPT_JWS_TYPE,
        kid: &kid,
        iss: issuer_url,
    })
    .map_err(|e| AppError::internal(format!("grant receipt header: {e}")))?;
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.as_bytes()),
        URL_SAFE_NO_PAD.encode(payload.as_bytes())
    ))
}

/// `payload` (the stored receipt bytes) as a compact JWS signed by `signer`
/// under `issuer_url`.
///
/// # Errors
/// When the issuer URL has no host, or the signer fails.
pub async fn sign_receipt(
    signer: &dyn Signer,
    issuer_url: &str,
    payload: &str,
) -> AppResult<String> {
    let input = signing_input(issuer_url, payload)?;
    let signature = signer
        .sign(input.as_bytes())
        .await
        .map_err(|e| AppError::internal(format!("grant receipt signature: {e}")))?;
    Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

#[cfg(test)]
mod tests {
    use ring::signature::{UnparsedPublicKey, ED25519};

    use super::*;
    use crate::barrage_keys::{LocalSigner, CHECKPOINT_ORIGIN, TOOL_DEF_SIG_CONTEXT};

    const ISSUER: &str = "https://api.dravr.ai";

    #[test]
    fn the_header_is_in_its_fixed_order() {
        let input = signing_input(ISSUER, "{}").unwrap();
        let (header, payload) = input.split_once('.').unwrap();
        let header = String::from_utf8(URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
        assert_eq!(
            header,
            r#"{"alg":"EdDSA","typ":"health-grant-receipt+jws","kid":"did:web:api.dravr.ai#checkpoint","iss":"https://api.dravr.ai"}"#,
            "bug: the protected header is not the fixed one"
        );
        assert_eq!(
            URL_SAFE_NO_PAD.decode(payload).unwrap(),
            b"{}",
            "bug: the payload is not the stored bytes"
        );
        assert!(
            signing_input("not a url", "{}").is_err(),
            "bug: a hostless issuer named a key"
        );
    }

    #[test]
    fn the_three_message_families_are_disjoint() {
        let jws_input = signing_input(ISSUER, r#"{"receipt_version":0}"#).unwrap();
        let note = format!("{CHECKPOINT_ORIGIN}\n42\nroot\n");
        let tool_def = format!("{TOOL_DEF_SIG_CONTEXT}sha256:{}", "0".repeat(64));
        let first_line = |message: &str| message.split('\n').next().unwrap_or_default().to_owned();
        assert!(
            jws_input.starts_with("eyJ") && !jws_input.contains('\n'),
            "bug: a JWS input can be read as a line-framed message: {jws_input}"
        );
        assert_ne!(
            first_line(&jws_input),
            CHECKPOINT_ORIGIN,
            "bug: a JWS input reads as a checkpoint note"
        );
        assert_ne!(
            first_line(&jws_input),
            first_line(TOOL_DEF_SIG_CONTEXT),
            "bug: a JWS input reads as a tool definition"
        );
        assert_ne!(
            first_line(&note),
            first_line(&tool_def),
            "bug: a checkpoint note reads as a tool definition"
        );
        for other in [&note, &tool_def] {
            assert!(
                !other.starts_with("eyJ") && other.contains('\n'),
                "bug: a line-framed message reads as a JWS input: {other}"
            );
        }
    }

    #[tokio::test]
    async fn a_receipt_verifies_under_the_signer_key() {
        let signer = LocalSigner::random().unwrap();
        let jws = sign_receipt(&signer, ISSUER, r#"{"receipt_version":0}"#)
            .await
            .unwrap();
        let (input, signature) = jws.rsplit_once('.').unwrap();
        UnparsedPublicKey::new(&ED25519, signer.public_key())
            .verify(
                input.as_bytes(),
                &URL_SAFE_NO_PAD.decode(signature).unwrap(),
            )
            .expect("the receipt verifies under the checkpoint key");
        assert_eq!(jws.split('.').count(), 3, "bug: not a compact JWS");
    }
}
