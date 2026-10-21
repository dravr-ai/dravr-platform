// ABOUTME: Content coverage for the invitation email body
// ABOUTME: Asserts the accept link, its expiry and attribute escaping are rendered, not merely that a String came back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The invitation body carries a single-use accept link into an `href`, so
//! these assert on the rendered content: a template that returned an empty
//! string, dropped the expiry, or interpolated the URL raw would fail them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_email::templates::invitation_html;

const ACCEPT_URL: &str = "https://app.example.test/accept-invite#token=Sel1.Ver2";

#[test]
fn invitation_renders_the_accept_link_as_the_call_to_action() {
    let html = invitation_html(ACCEPT_URL, 7);

    assert!(
        html.contains(r#"href="https://app.example.test/accept-invite#token=Sel1.Ver2""#),
        "the accept URL, token included, must be the anchor's href: {html}"
    );
    assert!(
        html.contains("Create your account"),
        "the call to action must name what the link does: {html}"
    );
    assert!(
        html.contains("You're invited"),
        "the body must say what the mail is: {html}"
    );
}

#[test]
fn invitation_states_that_the_link_is_single_use_and_when_it_expires() {
    let html = invitation_html(ACCEPT_URL, 7);
    assert!(
        html.contains("This link works once and expires in 7 days"),
        "the copy must state the expiry it was given: {html}"
    );

    let html = invitation_html(ACCEPT_URL, 3);
    assert!(
        html.contains("expires in 3 days"),
        "the expiry is the argument, not a fixed sentence: {html}"
    );
}

#[test]
fn a_quote_in_the_url_cannot_break_out_of_the_href_attribute() {
    let html = invitation_html(r#"https://evil.test/" onmouseover="steal()"#, 7);

    assert!(
        !html.contains(r#"" onmouseover="steal()"#),
        "a double quote in the URL must not close the attribute: {html}"
    );
    assert!(
        html.contains("&quot;"),
        "the quote must be encoded rather than dropped: {html}"
    );
}
