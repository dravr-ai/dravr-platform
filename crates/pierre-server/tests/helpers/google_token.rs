// ABOUTME: A Google-shaped signing identity for tests — an openssl-generated RSA key and certificate, tokens minted with it
// ABOUTME: Serves the kid → certificate map the verifier reads, so a test authenticates a Cloud Tasks delivery without Google
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

//! What a Cloud Tasks delivery looks like from the inside.
//!
//! Google signs the OIDC token a task carries with a key whose X.509
//! certificate it publishes under a key id. The verifier reads that map and
//! nothing else, so a test can be Google: generate a key and a self-signed
//! certificate for it, serve the map from a local listener, and mint tokens
//! with the key. The certificate is produced by the `openssl` CLI, present on
//! every developer machine and every CI runner this suite runs on; the
//! `rsa` crate can make the key but not the certificate the cache parses.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
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

/// One signing identity: a private key, the certificate that vouches for it,
/// and the key id the map serves it under.
pub struct TestSigner {
    private_pem: String,
    cert_pem: String,
    /// The `kid` this signer's certificate is published under.
    pub kid: String,
}

impl TestSigner {
    /// Generate a fresh 2048-bit key and a one-day self-signed certificate.
    #[must_use]
    pub fn generate() -> Self {
        let dir = env::temp_dir().join(format!("pierre-test-signer-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");
        let key = dir.join("key.pem");
        let cert = dir.join("cert.pem");
        let status = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=turn-runner-test",
                "-keyout",
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .expect("openssl is on PATH");
        assert!(
            status.status.success(),
            "openssl could not mint the test certificate: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        let private_pem = fs::read_to_string(&key).expect("private key");
        let cert_pem = fs::read_to_string(&cert).expect("certificate");
        let _ = fs::remove_dir_all(&dir);
        Self {
            private_pem,
            cert_pem,
            kid: format!("test-kid-{}", Uuid::new_v4().simple()),
        }
    }

    /// The certificate PEM, as the map serves it.
    #[must_use]
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// Mint `claims` under this signer's key and key id.
    #[must_use]
    pub fn mint(&self, claims: &GoogleClaims) -> String {
        self.mint_with_kid(claims, &self.kid)
    }

    /// Mint `claims` under this signer's key but a chosen key id — the way to
    /// present a token the published map cannot vouch for.
    #[must_use]
    pub fn mint_with_kid(&self, claims: &GoogleClaims, kid: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_owned());
        self.mint_with_header(claims, &header)
    }

    /// Mint `claims` with no key id in the header at all — a token no
    /// certificate map could ever vouch for.
    #[must_use]
    pub fn mint_without_kid(&self, claims: &GoogleClaims) -> String {
        self.mint_with_header(claims, &Header::new(Algorithm::RS256))
    }

    fn mint_with_header(&self, claims: &GoogleClaims, header: &Header) -> String {
        let key = EncodingKey::from_rsa_pem(self.private_pem.as_bytes()).expect("PEM key");
        encode(header, claims, &key).expect("token")
    }

    /// Serve this signer's `kid` → certificate map from a local listener and
    /// return its URL, the shape of `https://www.googleapis.com/oauth2/v1/certs`.
    pub async fn serve_certs(&self) -> String {
        let mut map = HashMap::new();
        map.insert(self.kid.clone(), self.cert_pem.clone());
        serve_cert_map(map).await
    }
}

/// Serve an arbitrary `kid` → certificate map from a local listener.
pub async fn serve_cert_map(map: HashMap<String, String>) -> String {
    async fn handler(State(map): State<Arc<HashMap<String, String>>>) -> impl IntoResponse {
        (
            [(CACHE_CONTROL, "public, max-age=3600, must-revalidate")],
            Json(map.as_ref().clone()),
        )
    }
    let app = Router::new()
        .route("/certs", get(handler))
        .with_state(Arc::new(map));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}/certs")
}
