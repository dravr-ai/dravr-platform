// ABOUTME: Guards the press client's both-or-neither wiring of URL and audience
// ABOUTME: A URL without an audience would post requests the press refuses, silently costing every chart

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
