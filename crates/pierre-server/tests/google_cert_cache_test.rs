// ABOUTME: Pins the Google certificate cache every Google-signed ID-token verifier reads its keys from
// ABOUTME: A local listener serves a real Google certificate map; the cache is driven without reaching Google
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre_auth::google_certs::GoogleCertCache` sits under Firebase social
//! login today and under every future Google-signed token check. Its
//! contract: one fetch per cache lifetime, an RSA public key PEM out of each
//! X.509 certificate in the map, a refetch before refusing an unknown key
//! id, and a typed refusal when the map yields no key at all. Each is pinned
//! here against a local stand-in for the certificate endpoint.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use pierre_auth::google_certs::{
    convert_certs_to_keys, extract_public_key_from_cert, parse_max_age, GoogleCertCache,
    DEFAULT_CACHE_TTL_SECS,
};
use pierre_core::errors::ErrorCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// One certificate as served at `https://www.googleapis.com/oauth2/v1/certs`
/// on 2026-09-05, under key id `943a3a5d7d919625a454e489b75c29adab57acba`.
/// Key extraction reads the certificate's subject public key and never its
/// validity window, so the fixture does not age.
const GOOGLE_KID: &str = "943a3a5d7d919625a454e489b75c29adab57acba";
const GOOGLE_CERT: &str = "-----BEGIN CERTIFICATE-----\n\
MIIDJzCCAg+gAwIBAgIJAJv8YZctwxPMMA0GCSqGSIb3DQEBBQUAMDYxNDAyBgNV\n\
BAMMK2ZlZGVyYXRlZC1zaWdub24uc3lzdGVtLmdzZXJ2aWNlYWNjb3VudC5jb20w\n\
HhcNMjYwMzMxMjI0OTEwWhcNMjcwMzMxMjI0OTEwWjA2MTQwMgYDVQQDDCtmZWRl\n\
cmF0ZWQtc2lnbm9uLnN5c3RlbS5nc2VydmljZWFjY291bnQuY29tMIIBIjANBgkq\n\
hkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEApIpnzA2ezyEERJSxiqpLBmMeIqATH+V6\n\
iuBtKIibXEyYovujrx8niqTeO6RIyXT6uDUUv0V2kJ8V/iWYFxzXY1BqK9IfcAmj\n\
g0XUDoyTVkoyLsF0gj299LH+zw5vCvy8jmamFIZKAbKcQ5hpHvSittM1vl+6vVL+\n\
i2GxyGbMA9aY6Hq15NylS1t7ELTYfQimlnvxcb7/DM0cuS5U1SfbCZMCpKhh0nrS\n\
lYds240oxpCJOV2rBahs/Ea5c7tezS1nwVC9W/E+bR9TF6BHkC/fv+E8DcWfkI/6\n\
geaJzBhINNxBfjx+w1+WUp2Jz3YYFWEfeQjxMqu+Fg6cGxwk7V16uQIDAQABozgw\n\
NjAMBgNVHRMBAf8EAjAAMA4GA1UdDwEB/wQEAwIHgDAWBgNVHSUBAf8EDDAKBggr\n\
BgEFBQcDAjANBgkqhkiG9w0BAQUFAAOCAQEAM775NpEtQ5iTVgLNZmzkwy1fgi9d\n\
CAkZdJs0K14rR+4VBGhrua0HOSS7dZv+3MBkLIP/2B6s+enQgW26nuNx3xlJKQmk\n\
D82sGZ0eGmXSS0U6Lzwr31zRKVm0wFbJU7Q/0LDpQ9gfnheZ7K9hrkuHOlr+EQtG\n\
jFJRHidxK7m+e/ogbEWgqzLNSAv60USs8mcZmBNLRWTKuAfisVm3v6FKALCv3bSC\n\
Nc1d1kzytBRaDsgZgy5IxOpbvy79vD9FbMjB6ZcjRrIKaU8TFAkLY1D14EZSB5oQ\n\
GVqAoYmgQ6xs6Im9HRFfLPTyL54D/narTMqIGR6G6jcV4/Y0MQqJP1M7yQ==\n\
-----END CERTIFICATE-----\n";

/// The certificate map a stand-in endpoint serves, and how often it was asked.
struct CertStub {
    body: Value,
    hits: AtomicUsize,
}

async fn serve(stub: Arc<CertStub>) -> String {
    async fn handler(State(stub): State<Arc<CertStub>>) -> impl IntoResponse {
        stub.hits.fetch_add(1, Ordering::SeqCst);
        (
            [(CACHE_CONTROL, "public, max-age=3600, must-revalidate")],
            Json(stub.body.clone()),
        )
    }
    let app = Router::new().route("/certs", get(handler)).with_state(stub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/certs")
}

fn google_map() -> Arc<CertStub> {
    Arc::new(CertStub {
        body: json!({ GOOGLE_KID: GOOGLE_CERT }),
        hits: AtomicUsize::new(0),
    })
}

#[tokio::test]
async fn a_certificate_map_yields_a_public_key_and_is_fetched_once() {
    let stub = google_map();
    let cache = GoogleCertCache::new(serve(Arc::clone(&stub)).await);

    let pem = cache.public_key(GOOGLE_KID).await.unwrap();
    assert!(
        pem.starts_with("-----BEGIN PUBLIC KEY-----\n"),
        "the certificate's subject public key comes out as PEM, got: {pem}"
    );
    assert!(pem.trim_end().ends_with("-----END PUBLIC KEY-----"));
    assert!(
        pem.lines().skip(1).all(|line| line.len() <= 64),
        "PEM body is wrapped at 64 columns"
    );

    let again = cache.public_key(GOOGLE_KID).await.unwrap();
    assert_eq!(again, pem);
    assert_eq!(
        stub.hits.load(Ordering::SeqCst),
        1,
        "a fresh cache is served without a second fetch"
    );
}

#[tokio::test]
async fn an_unknown_key_id_is_refetched_then_refused() {
    let stub = google_map();
    let cache = GoogleCertCache::new(serve(Arc::clone(&stub)).await);
    cache.public_key(GOOGLE_KID).await.unwrap();

    // A key id the cached map lacks could be a rotation the cache has not
    // seen, so the endpoint is asked once more before the token is refused.
    let err = cache.public_key("rotated-away").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(err.message, "Unknown token signing key");
    assert_eq!(
        stub.hits.load(Ordering::SeqCst),
        2,
        "one refetch for the unknown id, not one per call"
    );
}

#[tokio::test]
async fn a_map_with_no_usable_certificate_is_refused() {
    let stub = Arc::new(CertStub {
        body: json!({ "kid-1": "-----BEGIN CERTIFICATE-----\nnot a certificate\n-----END CERTIFICATE-----" }),
        hits: AtomicUsize::new(0),
    });
    let cache = GoogleCertCache::new(serve(stub).await);

    let err = cache.public_key("kid-1").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError);
    assert!(
        err.message.contains("No valid Google public keys"),
        "got: {}",
        err.message
    );
}

#[test]
fn convert_keeps_the_parsable_certificates_and_names_the_rest() {
    let mut certs = HashMap::new();
    certs.insert(GOOGLE_KID.to_owned(), GOOGLE_CERT.to_owned());
    certs.insert("broken".to_owned(), "garbage".to_owned());

    let keys = convert_certs_to_keys(certs).unwrap();
    assert_eq!(
        keys.len(),
        1,
        "the broken certificate is skipped, not fatal"
    );
    assert!(keys[GOOGLE_KID].starts_with("-----BEGIN PUBLIC KEY-----"));

    let err = convert_certs_to_keys([("only".to_owned(), "garbage".to_owned())]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError);
}

#[test]
fn extract_refuses_a_certificate_that_does_not_parse() {
    let err = extract_public_key_from_cert(
        "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError);
    assert!(extract_public_key_from_cert(GOOGLE_CERT).is_ok());
}

#[test]
fn max_age_is_read_from_cache_control() {
    assert_eq!(
        parse_max_age("public, max-age=3600, must-revalidate"),
        Some(3600)
    );
    assert_eq!(parse_max_age("max-age=19125"), Some(19_125));
    assert_eq!(parse_max_age("no-store"), None);
    assert_eq!(parse_max_age("max-age=soon"), None);
    assert_eq!(
        DEFAULT_CACHE_TTL_SECS, 3600,
        "the fallback when the header is absent"
    );
}
