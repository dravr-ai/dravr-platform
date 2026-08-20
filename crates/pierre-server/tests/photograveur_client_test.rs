// ABOUTME: Guards the press client's both-or-neither wiring of URL and audience
// ABOUTME: A URL without an audience would post requests the press refuses, silently costing every chart

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! The press degrades by design: any failure drops the chart and sends the
//! coach's prose. That is right for a cold start or a timeout, and wrong for a
//! misconfiguration, which would present as charts quietly never appearing
//! while every log line looks healthy. These assert the client refuses to be
//! half-configured in the first place.

use std::env;

use pierre_mcp_server::services::photograveur_client::{
    PhotograveurClient, PHOTOGRAVEUR_AUDIENCE_ENV, PHOTOGRAVEUR_URL_ENV,
};
use reqwest::Client;
use serial_test::serial;

/// Clear both variables so each case starts from a known environment.
fn clear() {
    env::remove_var(PHOTOGRAVEUR_URL_ENV);
    env::remove_var(PHOTOGRAVEUR_AUDIENCE_ENV);
}

#[test]
#[serial]
fn a_url_without_an_audience_disables_the_press() {
    clear();
    env::set_var(PHOTOGRAVEUR_URL_ENV, "https://press.example.a.run.app");

    let client = PhotograveurClient::from_env(Client::new());

    // Not merely "does not crash": enabled would mean the fidelity negotiator
    // offers messaging a chart, the press refuses the unsigned request, and the
    // athlete gets prose with no trace of why.
    assert!(
        !client.is_enabled(),
        "a press configured with a URL but no audience must report itself disabled"
    );
    clear();
}

#[test]
#[serial]
fn both_present_enables_the_press() {
    clear();
    env::set_var(PHOTOGRAVEUR_URL_ENV, "https://press.example.a.run.app");
    env::set_var(PHOTOGRAVEUR_AUDIENCE_ENV, "dravr-photograveur-development");

    let client = PhotograveurClient::from_env(Client::new());

    assert!(
        client.is_enabled(),
        "a fully configured press must be enabled"
    );
    clear();
}

#[test]
#[serial]
fn neither_present_leaves_the_press_off() {
    clear();

    let client = PhotograveurClient::from_env(Client::new());

    // The developer default: the stack runs identically without the service.
    assert!(!client.is_enabled());
}

#[test]
#[serial]
fn the_debug_impl_does_not_expose_the_token_source() {
    clear();
    env::set_var(PHOTOGRAVEUR_URL_ENV, "https://press.example.a.run.app");
    env::set_var(PHOTOGRAVEUR_AUDIENCE_ENV, "dravr-photograveur-development");

    let rendered = format!("{:?}", PhotograveurClient::from_env(Client::new()));

    // The client holds a live identity token once minted. Any log line that
    // formats it must not carry that token, which is why Debug is hand-written.
    assert!(
        rendered.contains("<configured>"),
        "expected the token source to be summarised, got: {rendered}"
    );
    assert!(
        !rendered.contains("IdTokenSource"),
        "Debug leaked the token source internals: {rendered}"
    );
    clear();
}

/// A loopback press needs no audience to be usable.
///
/// A developer runs the press on 127.0.0.1 with no Google identity in reach.
/// Under the both-or-neither rule that URL disabled the press outright, so a
/// local stack could never render a chart at all.
#[test]
#[serial]
fn a_loopback_url_enables_the_press_without_an_audience() {
    clear();
    env::set_var(PHOTOGRAVEUR_URL_ENV, "http://127.0.0.1:8092");

    let client = PhotograveurClient::from_env(Client::new());

    assert!(
        client.is_enabled(),
        "a press on loopback must be usable without an audience"
    );
    clear();
}

/// Enabled is not enough — the press call itself has to accept the missing
/// token rather than erroring on it.
///
/// `press()` used to demand a token source unconditionally, so a loopback press
/// reported itself enabled, the negotiator offered Slack an image URL, and
/// every fetch of that URL 500'd with "no identity-token source" while the
/// startup log announced the unauthenticated mode it never actually took
/// (2026-08-20). Asserting on the *transport* error proves the auth check was
/// passed: nothing is listening on this port, so reaching a connection failure
/// means the request was built and sent.
#[tokio::test]
#[serial]
async fn a_loopback_press_call_is_not_refused_for_lacking_a_token() {
    clear();
    // Port 1 is reserved and nothing binds it, so the connection always fails.
    env::set_var(PHOTOGRAVEUR_URL_ENV, "http://127.0.0.1:1");
    let client = PhotograveurClient::from_env(Client::new());

    let block = photograveur::RenderBlock::Table(photograveur::TableView {
        title: Some("Semaine".to_owned()),
        columns: vec!["Jour".to_owned(), "Distance".to_owned()],
        rows: vec![vec!["Mar".to_owned(), "12 km".to_owned()]],
        alignments: vec![
            photograveur::ColumnAlignment::Left,
            photograveur::ColumnAlignment::Right,
        ],
        source_tool: "get_activities".to_owned(),
    });
    let message = match client.press(&block, "dark").await {
        Ok(bytes) => {
            clear();
            panic!(
                "nothing is listening on port 1, so the press cannot succeed; got {} bytes",
                bytes.len()
            );
        }
        Err(e) => e.to_string(),
    };

    assert!(
        !message.contains("identity-token source"),
        "a loopback press must not be refused for lacking a token: {message}"
    );
    assert!(
        message.contains("unreachable"),
        "expected the transport failure that proves the request was sent: {message}"
    );
    clear();
}

/// The bypass must stay pinned to loopback. A remote host without an audience
/// is still a misconfiguration, not an invitation to post unsigned requests.
#[test]
#[serial]
fn a_remote_host_never_takes_the_loopback_bypass() {
    for url in [
        "https://press.example.a.run.app",
        // Not loopback despite the substring — the check reads the host, not
        // the whole URL.
        "https://127.0.0.1.evil.example.com",
        "http://10.0.0.230:8092",
    ] {
        clear();
        env::set_var(PHOTOGRAVEUR_URL_ENV, url);

        let client = PhotograveurClient::from_env(Client::new());

        assert!(
            !client.is_enabled(),
            "{url} must not be treated as loopback and must stay disabled without an audience"
        );
    }
    clear();
}
