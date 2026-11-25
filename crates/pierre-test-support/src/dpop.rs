// ABOUTME: Two P-256 client keys and the DPoP proofs (RFC 9449) a test signs with them
// ABOUTME: Shared by the server's DPoP suite and the live Redis replay test; never in a shipped crate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A client's `DPoP` key pair and the proofs it signs.
//!
//! [`A`] is the key tronc's own `DPoP` tests sign with; [`B`] is a second
//! valid key, for a proof by a key a grant is not bound to.

use dravr_tronc::dates::Timestamp;
use dravr_tronc::dpop::{access_token_hash, jwk_thumbprint, DpopError};
use jsonwebtoken::errors::Error as JwtError;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use uuid::Uuid;

const KEY_A: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n", // secret-scan-ok: tronc's published DPoP test key, no deployment trusts it
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgpbH6oQSVSYUgbrn3\n",
    "4AOEqaR1OtL0XIwAXxhiGRvRpcOhRANCAAR8xBbzOIvvGO0sqN5xb5q6xtZc01ze\n",
    "isqVehX/E2QN6niEKvzcpz1bCIJu3Lu5xmKN7H+YO77aKTQALV2uwDtD\n",
    "-----END PRIVATE KEY-----",
);
const KEY_A_X: &str = "fMQW8ziL7xjtLKjecW-ausbWXNNc3orKlXoV_xNkDeo";
const KEY_A_Y: &str = "eIQq_NynPVsIgm7cu7nGYo3sf5g7vtopNAAtXa7AO0M";

const KEY_B: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n", // secret-scan-ok: a throwaway P-256 test key, no deployment trusts it
    "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgr5m9QR1v+//M2thn\n",
    "bM7wJ83pAnAulYmrToyaZe7GRU6hRANCAAS7Q6COKJGHKRIuXNVnLUaAjQkNuZV0\n",
    "Tazd+Ydv7TGK4EJcVrKbLx4aMdh7tEo9ESKBRk/qq+MwpI9nizdS6YaY\n",
    "-----END PRIVATE KEY-----",
);
const KEY_B_X: &str = "u0OgjiiRhykSLlzVZy1GgI0JDbmVdE2s3fmHb-0xiuA";
const KEY_B_Y: &str = "QlxWspsvHhox2Hu0Sj0RIoFGT-qr4zCkj2eLN1Lphpg";

/// A client's `DPoP` key pair.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pem: &'static str,
    x: &'static str,
    y: &'static str,
}

/// The client's key, as tronc's own `DPoP` tests sign with it.
pub const A: Key = Key {
    pem: KEY_A,
    x: KEY_A_X,
    y: KEY_A_Y,
};

/// Another key: valid proofs, but not the key a grant is bound to.
pub const B: Key = Key {
    pem: KEY_B,
    x: KEY_B_X,
    y: KEY_B_Y,
};

impl Key {
    /// The public key, as a proof's `jwk` header carries it.
    #[must_use]
    pub fn jwk(self) -> Value {
        json!({"kty": "EC", "crv": "P-256", "x": self.x, "y": self.y})
    }

    /// The key's RFC 7638 thumbprint, the `jkt` a grant is bound to.
    ///
    /// # Errors
    ///
    /// Never for these keys; tronc's error when a jwk lacks a member.
    pub fn thumbprint(self) -> Result<String, DpopError> {
        jwk_thumbprint(&self.jwk())
    }

    /// A proof for `htm` `htu`, bound to `token` when one is given, made
    /// `age` seconds ago, with a fresh `jti`.
    ///
    /// # Errors
    ///
    /// A key or claims jsonwebtoken cannot sign, which these never are.
    pub fn proof_aged(
        self,
        htm: &str,
        htu: &str,
        token: Option<&str>,
        age: i64,
    ) -> Result<String, JwtError> {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("dpop+jwt".to_owned());
        header.jwk = Some(serde_json::from_value(self.jwk())?);
        let mut claims = json!({
            "jti": Uuid::new_v4().to_string(),
            "htm": htm,
            "htu": htu,
            "iat": Timestamp::now().epoch_seconds() - age,
        });
        if let Some(token) = token {
            claims["ath"] = json!(access_token_hash(token));
        }
        let key = EncodingKey::from_ec_pem(self.pem.as_bytes())?;
        encode(&header, &claims, &key)
    }

    /// A proof for `htm` `htu` made now.
    ///
    /// # Errors
    ///
    /// As [`Self::proof_aged`].
    pub fn proof(self, htm: &str, htu: &str, token: Option<&str>) -> Result<String, JwtError> {
        self.proof_aged(htm, htu, token, 0)
    }
}
