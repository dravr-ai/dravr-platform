// ABOUTME: Pins the coach-import SSRF policy: literal blocks, resolved-address vetting,
// ABOUTME: and that the vetted set survives intact — it is what the connection pins to

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Coach import fetches a coach definition from a user-supplied URL, which is
//! the platform's one arbitrary-URL egress. Three layers guard it, tested here
//! from the outside in:
//!
//! - [`validate_url_security`] blocks bad schemes and hostile literal hosts
//!   before any network activity;
//! - [`vet_resolved_addrs`] is the pure address policy — every resolved
//!   address public, at least one present — and what it returns is **exactly
//!   the set the connection is pinned to** via `resolve_to_addrs`, which is
//!   what closes the DNS-rebinding window (registre#5): a record that flips to
//!   an internal address between the pre-flight lookup and the connect never
//!   gets re-resolved;
//! - [`fetch_markdown_from_url`] runs both before building the client, so a
//!   blocked URL fails without a single packet sent.

use std::net::SocketAddr;

use pierre_services::coach_import::{
    fetch_markdown_from_url, validate_url_security, vet_resolved_addrs,
};

fn addr(s: &str) -> SocketAddr {
    s.parse().expect("test literal parses")
}

// ── The pure address policy ─────────────────────────────────────────────────

#[test]
fn vetted_addresses_survive_intact_for_pinning() {
    let public = vec![addr("93.184.216.34:443"), addr("[2606:2800:220:1::1]:443")];
    let vetted = vet_resolved_addrs(public.clone()).expect("public addresses pass");
    assert_eq!(
        vetted, public,
        "the vetted set must be exactly what was checked, in order — it is \
         what the connection gets pinned to"
    );
}

#[test]
fn one_private_address_poisons_the_whole_set() {
    // A rebinding-style answer mixes a public address with an internal one;
    // serving the public one anyway would let a retry reach the internal.
    for private in [
        "127.0.0.1:443",
        "10.0.0.7:443",
        "172.16.3.4:443",
        "192.168.1.20:443",
        "169.254.169.254:443", // cloud metadata
        "[::1]:443",
    ] {
        let mixed = vec![addr("93.184.216.34:443"), addr(private)];
        let err = vet_resolved_addrs(mixed).expect_err("a private address must reject the set");
        assert!(
            err.to_string().contains("private or loopback"),
            "{private}: expected the private-address rejection, got: {err}"
        );
    }
}

#[test]
fn an_empty_resolution_is_an_error_not_an_open_pin() {
    let err = vet_resolved_addrs(Vec::new()).expect_err("no addresses must not pass");
    assert!(
        err.to_string().contains("did not resolve"),
        "expected the empty-resolution error, got: {err}"
    );
}

// ── Literal blocks, before any network activity ─────────────────────────────

#[test]
fn hostile_literals_are_blocked_pre_resolution() {
    for url in [
        "http://example.com/coach.md",         // scheme
        "https://127.0.0.1/coach.md",          // loopback literal
        "https://169.254.169.254/latest/meta", // cloud metadata literal
        "https://10.0.0.7/coach.md",           // RFC 1918 literal
        "https://localhost/coach.md",          // loopback by name
        "https://[::1]/coach.md",              // v6 loopback literal
    ] {
        assert!(
            validate_url_security(url).is_err(),
            "{url} must be rejected by the literal policy"
        );
    }
}

#[tokio::test]
async fn fetch_rejects_blocked_urls_without_touching_the_network() {
    // These fail in the pre-flight (scheme/literal policy), so the whole fetch
    // errors before a client is even built — no packet leaves the process.
    for url in ["https://169.254.169.254/x", "https://localhost/x"] {
        let err = fetch_markdown_from_url(url)
            .await
            .expect_err("blocked URL must not fetch");
        assert!(
            err.to_string().to_lowercase().contains("not allowed")
                || err.to_string().to_lowercase().contains("private"),
            "{url}: expected a policy rejection, got: {err}"
        );
    }
}
