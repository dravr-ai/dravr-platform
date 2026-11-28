// ABOUTME: The bearer a hosted sign-in page's calls carry, for suites that drive those routes in-process
// ABOUTME: A provider link-token is exchanged at the page load that burns it; the routes refuse the link itself (carnet#1874)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Only the suites that drive the hosted sign-in routes call it.
#![allow(dead_code)]

use pierre_middleware::provider_link_token::{
    mint_link_token, mint_signin_session_token, verify_link_token, MintProviderLinkTokenArgs,
};

/// The sign-in session token the hosted page embeds for the link `args` mints.
///
/// It is what the page's login, 2FA, OTP and cancel calls send as their bearer.
pub(crate) fn hosted_signin_session(
    args: &MintProviderLinkTokenArgs<'_>,
    admin_jwt_secret: &str,
) -> String {
    let link = mint_link_token(args, admin_jwt_secret).expect("mint hosted-login token");
    let claims = verify_link_token(&link, admin_jwt_secret, args.provider).expect("verify link");
    mint_signin_session_token(&claims, admin_jwt_secret).expect("mint sign-in session")
}

/// The bearer a rendered hosted page embeds in its script's config.
pub(crate) fn embedded_page_token(page: &str) -> &str {
    let start = page.find("token: \"").map_or_else(
        || panic!("the page embeds a token: {page}"),
        |at| at + "token: \"".len(),
    );
    let end = page[start..].find('"').expect("the token string closes");
    &page[start..start + end]
}
