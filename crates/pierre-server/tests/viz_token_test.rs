// ABOUTME: The signed chart URL must not be forgeable, tamperable, or replayable after expiry
// ABOUTME: This token is the only thing standing between a public URL and every athlete's charts

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! Why this is worth its own file.
//!
//! `/api/viz/{token}.png` is unauthenticated by necessity — the caller is a
//! messaging channel's media fetcher, which carries no session and no cookie.
//! The token in the path is therefore the entire authorisation, and every
//! property below is load-bearing: if any one of them fails, one leaked URL
//! becomes a way to enumerate every chart in every conversation.

use pierre_core::models::TenantId;
use pierre_mcp_server::routes::viz::{VizTarget, VizToken};
use pierre_mcp_server::services::messaging_ingress::viz_delivery::strip_viz_markers;
use uuid::Uuid;

const SECRET: &str = "test-secret-not-a-real-key";

fn target() -> VizTarget {
    VizTarget {
        conversation_id: "conv-1".to_owned(),
        user_id: "user-1".to_owned(),
        tenant_id: TenantId::from_uuid(Uuid::nil()),
        message_id: "msg-1".to_owned(),
    }
}

#[test]
fn a_minted_token_round_trips() {
    let minted = VizToken::for_block(target(), 2, "dark", "fr").mint(SECRET);
    let parsed = VizToken::verify(&minted, SECRET).expect("a freshly minted token must verify");

    assert_eq!(parsed.conversation_id, "conv-1");
    assert_eq!(parsed.user_id, "user-1");
    assert_eq!(parsed.message_id, "msg-1");
    assert_eq!(parsed.block_index, 2);
    assert_eq!(parsed.theme, "dark");
    assert_eq!(parsed.locale, "fr");
}

/// A different key must not verify.
///
/// This is the whole security property: without it, anyone who can guess the
/// payload format mints their own URLs.
#[test]
fn a_token_signed_with_another_key_is_refused() {
    let minted = VizToken::for_block(target(), 0, "dark", "fr").mint(SECRET);
    assert!(
        VizToken::verify(&minted, "a-different-secret").is_none(),
        "a token must not verify under a key that did not sign it"
    );
}

/// Every field is covered by the MAC, so editing any of them invalidates it.
///
/// The block index and message id are the dangerous ones — those are what an
/// attacker would walk to read someone else's charts.
#[test]
fn tampering_with_any_field_invalidates_the_signature() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    let minted = VizToken::for_block(target(), 0, "dark", "fr").mint(SECRET);
    let (encoded, signature) = minted.split_once('.').expect("token has two parts");
    let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();

    for (from, to, what) in [
        ("msg-1", "msg-2", "message id"),
        ("conv-1", "conv-2", "conversation id"),
        ("user-1", "user-2", "user id"),
        ("|0|", "|1|", "block index"),
    ] {
        let edited = payload.replace(from, to);
        assert_ne!(edited, payload, "the {what} fixture must actually change");
        let forged = format!("{}.{signature}", URL_SAFE_NO_PAD.encode(edited.as_bytes()));
        assert!(
            VizToken::verify(&forged, SECRET).is_none(),
            "editing the {what} must invalidate the token"
        );
    }
}

#[test]
fn an_expired_token_is_refused() {
    let mut token = VizToken::for_block(target(), 0, "dark", "fr");
    token.expires_at = chrono::Utc::now().timestamp() - 1;
    let minted = token.mint(SECRET);

    assert!(
        VizToken::verify(&minted, SECRET).is_none(),
        "a token past its expiry must not verify even though it is correctly signed"
    );
}

#[test]
fn a_token_expiring_soon_still_verifies() {
    let mut token = VizToken::for_block(target(), 0, "dark", "fr");
    token.expires_at = chrono::Utc::now().timestamp() + 30;
    assert!(
        VizToken::verify(&token.mint(SECRET), SECRET).is_some(),
        "a token inside its window must verify"
    );
}

#[test]
fn malformed_tokens_are_refused_rather_than_panicking() {
    for raw in [
        "",
        "no-dot",
        ".",
        "not-base64.not-base64",
        "AAAA.AAAA",
        "....",
    ] {
        assert!(
            VizToken::verify(raw, SECRET).is_none(),
            "{raw:?} must be refused"
        );
    }
}

/// Markers are for clients that interleave; a channel must never see one.
#[test]
fn markers_are_stripped_from_channel_prose() {
    let stripped =
        strip_viz_markers("Ta charge grimpe.\n\n⟦viz:0⟧\n\nOn coupe jeudi.\n\n⟦viz:1⟧\n\nÀ jeudi.");

    assert!(!stripped.contains("⟦viz:"), "a marker survived: {stripped}");
    assert!(stripped.contains("Ta charge grimpe."));
    assert!(stripped.contains("On coupe jeudi."));
    assert!(stripped.contains("À jeudi."));
    assert!(
        !stripped.contains("\n\n\n"),
        "removing a marker must not leave a triple newline: {stripped:?}"
    );
}

/// An unterminated marker is not a marker.
///
/// Swallowing from `⟦viz:` to end-of-string would drop the rest of the reply,
/// which is a far worse outcome than showing a stray bracket.
#[test]
fn an_unterminated_marker_does_not_eat_the_reply() {
    let stripped = strip_viz_markers("Avant ⟦viz:0 et la suite du message");
    assert!(
        stripped.contains("et la suite du message"),
        "the tail must survive: {stripped}"
    );
}

#[test]
fn prose_without_markers_is_unchanged_apart_from_trimming() {
    let text = "Une réponse ordinaire.\n\nDeux paragraphes.";
    assert_eq!(strip_viz_markers(text), text);
}
