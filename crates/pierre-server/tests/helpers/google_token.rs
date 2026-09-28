// ABOUTME: A Google-shaped signing identity for tests — an RSA key published as a JWK, tokens minted with it
// ABOUTME: Serves the JWK set the verifiers read and counts its fetches, so a test authenticates without Google
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

//! What a Google-signed token looks like from the inside.
//!
//! Google signs the OIDC token a Cloud Tasks task carries, and every Firebase
//! ID token, with a key whose modulus and exponent it publishes as a JWK under
//! a key id. The verifiers read that set and nothing else, so a test can be
//! Google: generate a key, serve its JWK from a local listener, and mint tokens
//! with the key. The listener counts its fetches, so a test can pin how often
//! a verifier goes back to Google.

use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use pierre_auth::admin::jwks::{JsonWebKey, JsonWebKeySet, RsaKeyPair};
use serde::Serialize;
use tokio::net::TcpListener;
use uuid::Uuid;

/// The claims a Cloud Tasks token carries, as the verifier reads them.
#[derive(Debug, Clone, Serialize)]
pub struct GoogleClaims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub exp: u64,
    pub iat: u64,
}

impl GoogleClaims {
    /// A token Cloud Tasks would mint for `audience` on behalf of `service_account`,
    /// valid for an hour.
    #[must_use]
    pub fn cloud_tasks(audience: &str, service_account: &str) -> Self {
        let now = now_secs();
        Self {
            iss: "https://accounts.google.com".to_owned(),
            aud: audience.to_owned(),
            sub: "115000000000000000001".to_owned(),
            email: Some(service_account.to_owned()),
            email_verified: Some(true),
            exp: now + 3600,
            iat: now,
        }
    }
}

/// Seconds since the epoch, as a JWT counts them.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs()
}

/// One signing identity: an RSA key and the key id its JWK is published under.
pub struct TestSigner {
    key: RsaKeyPair,
    encoding_key: EncodingKey,
    /// The `kid` this signer's JWK is published under.
    pub kid: String,
}

impl TestSigner {
    /// Generate a fresh 2048-bit key under a fresh key id.
    #[must_use]
    pub fn generate() -> Self {
        let kid = format!("test-kid-{}", Uuid::new_v4().simple());
        let key = RsaKeyPair::generate_with_key_size(&kid, 2048).expect("RSA key");
        let encoding_key = key.encoding_key().expect("encoding key");
        Self {
            key,
            encoding_key,
            kid,
        }
    }

    /// This signer's public key as the JWK set publishes it: modulus and
    /// exponent, base64url.
    #[must_use]
    pub fn jwk(&self) -> JsonWebKey {
        self.key.to_jwk().expect("JWK")
    }

    /// Mint `claims` under this signer's key and key id.
    #[must_use]
    pub fn mint(&self, claims: &impl Serialize) -> String {
        self.mint_with_kid(claims, &self.kid)
    }

    /// Mint `claims` under this signer's key but a chosen key id — the way to
    /// present a token the published set cannot vouch for.
    #[must_use]
    pub fn mint_with_kid(&self, claims: &impl Serialize, kid: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_owned());
        self.mint_with_header(claims, &header)
    }

    /// Mint `claims` with no key id in the header at all — a token no key set
    /// could ever vouch for.
    #[must_use]
    pub fn mint_without_kid(&self, claims: &impl Serialize) -> String {
        self.mint_with_header(claims, &Header::new(Algorithm::RS256))
    }

    fn mint_with_header(&self, claims: &impl Serialize, header: &Header) -> String {
        encode(header, claims, &self.encoding_key).expect("token")
    }

    /// Serve this signer's JWK from a local listener, the shape of
    /// `https://www.googleapis.com/oauth2/v3/certs`.
    pub async fn serve_jwks(&self) -> KeySetServer {
        serve_key_set(vec![self.jwk()]).await
    }
}

/// A local stand-in for a Google JWK set endpoint.
pub struct KeySetServer {
    /// Where the set is served.
    pub url: String,
    fetches: Arc<AtomicUsize>,
}

impl KeySetServer {
    /// How many times the set has been fetched.
    #[must_use]
    pub fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

/// Serve `keys` as a JWK set from a local listener, counting every fetch.
pub async fn serve_key_set(keys: Vec<JsonWebKey>) -> KeySetServer {
    /// The set to serve and the fetch counter, shared with every request task.
    struct Served {
        set: JsonWebKeySet,
        fetches: Arc<AtomicUsize>,
    }
    async fn handler(State(served): State<Arc<Served>>) -> Json<JsonWebKeySet> {
        served.fetches.fetch_add(1, Ordering::SeqCst);
        Json(served.set.clone())
    }
    let fetches = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/certs", get(handler))
        .with_state(Arc::new(Served {
            set: JsonWebKeySet { keys },
            fetches: Arc::clone(&fetches),
        }));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    KeySetServer {
        url: format!("http://{addr}/certs"),
        fetches,
    }
}

/// A key-set URL nothing listens on: a fetch from it fails to connect.
#[must_use]
pub fn unreachable_key_set_url() -> String {
    let port = StdTcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("local addr")
        .port();
    format!("http://127.0.0.1:{port}/certs")
}
