// ABOUTME: Content coverage for the dravr.ai docs sign-in link email body
// ABOUTME: Asserts the link is the escaped call to action and the expiry is stated, not merely that a String came back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The sign-in body carries a single-use token in an `href` and states how long
//! it lives, so these assert on the rendered content: a template that returned
//! an empty string, dropped the TTL, or interpolated the URL raw would fail.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_email::templates::website_sign_in_html;

#[test]
fn the_sign_in_link_is_the_call_to_action_with_its_expiry() {
    let url = "https://dravr.ai/docs/auth/callback?token=abc.def&next=%2Fdocs&lang=en";
    let html = website_sign_in_html(url, 15);

    assert!(
        html.contains(
            r#"href="https://dravr.ai/docs/auth/callback?token=abc.def&amp;next=%2Fdocs&amp;lang=en""#
        ),
        "the sign-in URL must be the anchor's href, with & encoded: {html}"
    );
    assert!(
        html.contains("Sign in to the docs"),
        "the call to action must name what the link does: {html}"
    );
    assert!(
        html.contains("expires in 15 minutes"),
        "the body must state the link's lifetime: {html}"
    );
}

#[test]
fn a_quote_in_the_url_cannot_break_out_of_the_href_attribute() {
    let html = website_sign_in_html(r#"https://evil.test/" onmouseover="steal()"#, 15);

    assert!(
        !html.contains(r#"" onmouseover="steal()"#),
        "a double quote in the URL must not close the attribute: {html}"
    );
    assert!(
        html.contains("&quot;"),
        "the quote must be encoded rather than dropped: {html}"
    );
}
