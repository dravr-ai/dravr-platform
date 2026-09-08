// ABOUTME: Content coverage for the invitation email body
// ABOUTME: Asserts the sign-up link is rendered and attribute-escaped, not merely that a String came back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The invitation body is the one email carrying an operator-supplied URL into
//! an `href`, so these assert on the rendered content: a template that returned
//! an empty string, or one that interpolated the URL raw, would fail them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_email::templates::invitation_html;

#[test]
fn invitation_renders_the_sign_up_link_as_the_call_to_action() {
    let html = invitation_html("https://app.example.test/signup");

    assert!(
        html.contains(r#"href="https://app.example.test/signup""#),
        "the sign-up URL must be the anchor's href: {html}"
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
fn a_quote_in_the_url_cannot_break_out_of_the_href_attribute() {
    let html = invitation_html(r#"https://evil.test/" onmouseover="steal()"#);

    assert!(
        !html.contains(r#"" onmouseover="steal()"#),
        "a double quote in the URL must not close the attribute: {html}"
    );
    assert!(
        html.contains("&quot;"),
        "the quote must be encoded rather than dropped: {html}"
    );
}
