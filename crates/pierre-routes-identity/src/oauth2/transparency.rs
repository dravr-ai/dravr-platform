// ABOUTME: Dravr's checkpoint key as a did:web document, and the newest signed record_log checkpoint, for any verifier
// ABOUTME: Public and uncached, like the JWKS: a witness or an athlete checks a checkpoint without an account
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Transparency documents (carnet#1542, design rows 20 and 67).
//!
//! - `GET /.well-known/did.json`: `did:web:<issuer host>` naming the Ed25519
//!   key `record_log` checkpoints are signed with, as a `JsonWebKey2020`
//!   (`kty: OKP`, `crv: Ed25519`), and its C2SP key name. `404` while this
//!   deployment signs no checkpoint.
//! - `GET /.well-known/dravr-checkpoint`: the newest signed checkpoint note
//!   that anchors the deletion receipt chain (C2SP signed-note text), for a
//!   witness to co-sign or an athlete to compare with their export. `404`
//!   before the first one.

use std::future::{ready, Ready};

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use pierre_services::barrage_keys::{
    checkpoint_did, checkpoint_key_id, published_checkpoint_key, CheckpointKey,
};
use serde_json::{json, Value};
use tracing::error;

use super::{OAuth2Context, OAuth2Routes};

impl OAuth2Routes {
    /// `did:web` document naming the checkpoint signing key: built from the
    /// key published at boot, with nothing to wait on.
    pub(super) fn handle_did_document(State(context): State<OAuth2Context>) -> Ready<Response> {
        ready(did_document(&context))
    }
}

/// The `did:web` document of the published checkpoint key, or `404`.
fn did_document(context: &OAuth2Context) -> Response {
    published_checkpoint_key()
        .and_then(|key| did_document_json(key, &context.config.issuer_url))
        .map_or_else(
            || StatusCode::NOT_FOUND.into_response(),
            |document| Json(document).into_response(),
        )
}

/// The `did:web` document naming `key` under `issuer_url`'s host; `None`
/// when the issuer URL has no host.
fn did_document_json(key: &CheckpointKey, issuer_url: &str) -> Option<Value> {
    let did = checkpoint_did(issuer_url)?;
    let method = checkpoint_key_id(issuer_url)?;
    Some(json!({
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/suites/jws-2020/v1"
        ],
        "id": did,
        "verificationMethod": [{
            "id": method,
            "type": "JsonWebKey2020",
            "controller": did,
            "publicKeyJwk": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": URL_SAFE_NO_PAD.encode(key.public_key),
            },
            "c2spKeyName": key.name,
        }],
        "assertionMethod": [method],
    }))
}

impl OAuth2Routes {
    /// The newest signed checkpoint note.
    pub(super) async fn handle_latest_checkpoint(State(context): State<OAuth2Context>) -> Response {
        match context
            .database
            .repositories()
            .account_deletion
            .latest_anchor()
            .await
        {
            Ok(Some(anchor)) => (
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                anchor.note,
            )
                .into_response(),
            Ok(None) => StatusCode::NOT_FOUND.into_response(),
            Err(e) => {
                error!(error = %e, "Failed to read the newest checkpoint");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use pierre_services::barrage_keys::CheckpointKey;

    use super::did_document_json;

    #[test]
    fn the_did_document_names_the_checkpoint_key_as_an_ed25519_jwk() {
        let key = CheckpointKey {
            name: "dravr.ai/barrage-log".to_owned(),
            public_key: [7; 32],
        };
        let document = did_document_json(&key, "https://api.dravr.ai")
            .expect("an issuer with a host has a did document");
        let method = &document["verificationMethod"][0];
        assert_eq!(
            document["id"], "did:web:api.dravr.ai",
            "bug: the document is not the issuer's did"
        );
        assert_eq!(
            method["id"], "did:web:api.dravr.ai#checkpoint",
            "bug: the method is not under the did"
        );
        assert_eq!(
            document["assertionMethod"][0], method["id"],
            "bug: the checkpoint key cannot assert"
        );
        assert_eq!(
            method["publicKeyJwk"]["x"],
            URL_SAFE_NO_PAD.encode([7_u8; 32]),
            "bug: the JWK is not the published key"
        );
        assert_eq!(
            method["publicKeyJwk"]["crv"], "Ed25519",
            "bug: the key is not named Ed25519"
        );
        assert_eq!(
            method["c2spKeyName"], "dravr.ai/barrage-log",
            "bug: a verifier cannot match the note's key name"
        );
        assert!(
            did_document_json(&key, "not a url").is_none(),
            "bug: a document without a host"
        );
    }
}
