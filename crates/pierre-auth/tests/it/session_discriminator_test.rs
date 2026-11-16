// ABOUTME: A token is the athlete's session by the client it names and then by its grant
// ABOUTME: Pins which tokens the session validators take: a delegation holding every scope is not one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A session is Dravr's own app signed in: a token naming no client or one
//! of Dravr's apps, holding the whole self grant. A token naming any other
//! client is that client's, whatever it holds. The authorization server mints
//! no application a token holding every scope, since `admin` is not
//! delegated; these tests mint one to pin what it would be (carnet#1511).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::LazyLock;

use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::{AuthManager, Claims, DelegatedGrant};
use pierre_auth::oauth2_server::first_party::{MOBILE_CLIENT_ID, WEB_CLIENT_ID};
use pierre_core::constants::service_names::OAUTH2_AUTHORIZE;
use pierre_core::errors::ErrorCode;
use pierre_core::models::User;
use pierre_core::permissions::scopes::OAuthScope;
use uuid::Uuid;

/// A third-party client, as dynamic registration names one.
const APPLICATION: &str = "mcp_client_test";

/// One signing key for the suite: generating an RSA key is the slow part.
static JWKS: LazyLock<JwksManager> = LazyLock::new(|| {
    let mut jwks = JwksManager::new();
    jwks.generate_rsa_key_pair_with_size("session-discriminator", 2048)
        .unwrap();
    jwks
});

fn auth() -> AuthManager {
    AuthManager::new(24)
}

fn athlete() -> User {
    User::new(
        "athlete@example.com".to_owned(),
        "not-a-password-hash".to_owned(),
        Some("Athlete".to_owned()),
    )
}

/// `claims` signed with the suite's key: a token no minter issues.
fn signed(claims: &Claims) -> String {
    let key = JWKS.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, claims, &key.encoding_key().unwrap()).unwrap()
}

fn claims_of(token: &str) -> Claims {
    insecure_decode::<Claims>(token).unwrap().claims
}

/// The session one of Dravr's apps holds, naming `client_id` if any.
fn session(client_id: Option<&str>) -> String {
    auth()
        .generate_client_session_token(&athlete(), &JWKS, None, client_id)
        .unwrap()
}

/// The access token `APPLICATION` holds, carrying `scopes`.
fn delegated(scopes: &[OAuthScope]) -> String {
    let scopes: Vec<String> = scopes
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();
    auth()
        .generate_oauth_access_token(
            &JWKS,
            &DelegatedGrant {
                client_id: APPLICATION,
                scopes: &scopes,
                audience: None,
            },
            &Uuid::new_v4(),
            &[],
            None,
        )
        .unwrap()
}

/// `token` naming no client, as one minted before the claim existed.
fn naming_no_client(token: &str) -> String {
    let mut claims = claims_of(token);
    claims.client_id = None;
    signed(&claims)
}

fn refused_as_no_session(token: &str, what: &str) {
    let refused = auth().validate_session_token(token, &JWKS).expect_err(what);
    assert_eq!(refused.code, ErrorCode::AuthInvalid, "{what}");
}

/// The two predicates over the claims every other test mints.
#[test]
fn a_token_is_a_session_by_the_client_it_names_and_then_by_its_grant() {
    for client_id in [Some(WEB_CLIENT_ID), Some(MOBILE_CLIENT_ID), None] {
        let claims = claims_of(&session(client_id));
        assert!(!claims.is_delegated(), "{client_id:?}");
        assert!(claims.is_session(), "{client_id:?}");
    }

    for grant in [
        OAuthScope::default_grant(),
        OAuthScope::delegable_grant(),
        OAuthScope::self_grant(),
    ] {
        let claims = claims_of(&delegated(&grant));
        assert!(claims.is_delegated(), "{grant:?}");
        assert!(!claims.is_session(), "{grant:?}");
    }

    // Naming no client, a token is nobody's delegation by name; its grant
    // alone says whether it is a session.
    let unnamed = claims_of(&naming_no_client(
        &delegated(&OAuthScope::delegable_grant()),
    ));
    assert!(
        !unnamed.is_delegated(),
        "a token naming no client is no third party's by name"
    );
    assert!(
        !unnamed.is_session(),
        "a narrow grant is no session, whoever it names"
    );

    // A client's own token names the client, so it is the client's.
    let scopes = vec![OAuthScope::FitnessRead.as_str().to_owned()];
    let client_credentials = auth()
        .generate_client_credentials_token(&JWKS, APPLICATION, &scopes, None, None)
        .unwrap();
    let claims = claims_of(&client_credentials);
    assert!(
        claims.is_delegated(),
        "a client-credentials token is its client's"
    );
    assert!(
        !claims.is_session(),
        "a client-credentials token is no session"
    );

    // An impersonation token is a session by these two; its own claim is
    // what the session validators refuse it for.
    let impersonation = auth()
        .generate_impersonation_token(&athlete(), Uuid::new_v4(), "session-1", &JWKS, None)
        .unwrap();
    let claims = claims_of(&impersonation);
    assert!(
        !claims.is_delegated(),
        "an impersonation token names no client"
    );
    assert!(
        claims.is_session(),
        "an impersonation token has a session's client and grant"
    );
}

#[test]
fn a_session_validates_as_one_whichever_app_it_names() {
    for client_id in [Some(WEB_CLIENT_ID), Some(MOBILE_CLIENT_ID), None] {
        let claims = auth()
            .validate_session_token(&session(client_id), &JWKS)
            .unwrap_or_else(|e| panic!("{client_id:?}: a session validates as one: {e}"));
        assert_eq!(
            claims.client_id.as_deref(),
            client_id,
            "the validated claims name the app the session named"
        );
    }
}

/// Ids that are not exactly one of Dravr's apps: near misses, the empty
/// string, and clients as registration names them.
const NOT_DRAVRS_APPS: [&str; 9] = [
    "",
    " dravr-web",
    "dravr-web ",
    "Dravr-Web",
    "DRAVR-MOBILE",
    "dravr-web\n",
    "dravr_web",
    "unregistered-client",
    APPLICATION,
];

/// A session holds every scope, `admin` included, and a token naming a third
/// party is that party's delegation. The session minter therefore signs none
/// naming a client that is not one of Dravr's apps: it would be a delegation
/// holding a scope no delegation is granted.
#[test]
fn the_session_minter_names_no_third_party() {
    for client_id in NOT_DRAVRS_APPS {
        // Matched, never `expect_err`: that would print a minted token.
        match auth().generate_client_session_token(&athlete(), &JWKS, None, Some(client_id)) {
            Ok(token) => {
                let claims = claims_of(&token);
                panic!(
                    "{client_id:?}: the session minter signed a token that is a delegation \
                     ({}) holding {:?}",
                    claims.is_delegated(),
                    claims.scope
                );
            }
            Err(refused) => assert_eq!(refused.code, ErrorCode::InternalError, "{client_id:?}"),
        }
    }
}

#[test]
fn a_delegation_is_no_session_however_wide_its_grant() {
    refused_as_no_session(
        &delegated(&OAuthScope::default_grant()),
        "a narrow delegation is not a session",
    );
    refused_as_no_session(
        &delegated(&OAuthScope::delegable_grant()),
        "the widest delegation minted today is not a session",
    );
    refused_as_no_session(
        &delegated(&OAuthScope::self_grant()),
        "a delegation holding every scope is not a session",
    );
}

#[test]
fn a_narrow_grant_naming_no_client_is_no_session() {
    // The control for the re-signing helper: a session it re-signs still
    // validates, so the refusal below is for the grant, not the signature.
    auth()
        .validate_session_token(&naming_no_client(&session(Some(WEB_CLIENT_ID))), &JWKS)
        .expect("a re-signed session validates as one");
    refused_as_no_session(
        &naming_no_client(&delegated(&OAuthScope::delegable_grant())),
        "a narrow grant is not a session, whoever it names",
    );
}

/// An impersonation token has a session's shape (no client, every scope) but
/// for its own claim, and stays refused (carnet#1560).
#[test]
fn an_impersonation_token_is_no_session() {
    let token = auth()
        .generate_impersonation_token(&athlete(), Uuid::new_v4(), "session-1", &JWKS, None)
        .unwrap();
    let claims = claims_of(&token);
    assert_eq!(
        claims.client_id, None,
        "an impersonation token names no client"
    );
    assert!(
        OAuthScope::is_self_grant(&OAuthScope::parse_granted(&claims.scope)),
        "an impersonation token holds every scope"
    );
    refused_as_no_session(&token, "an impersonation token is not a session");
}

/// The authorization server's own sign-in cookie is read by the same rule: a
/// token addressed to it that an application holds is not its session.
#[test]
fn the_authorization_servers_sign_in_takes_no_delegation_either() {
    let own = auth()
        .generate_authorization_session_token(&athlete(), &JWKS, None)
        .unwrap();
    auth()
        .validate_authorization_session_token(&own, &JWKS)
        .expect("the authorization server's own sign-in token is its session");

    // The control for the re-signing helper: the same claims under a new
    // `jti` are still the sign-in session, so the refusals below are for
    // what the claims say, not for the signature.
    let mut reissued = claims_of(&own);
    reissued.jti = Uuid::new_v4().to_string();
    auth()
        .validate_authorization_session_token(&signed(&reissued), &JWKS)
        .expect("a re-signed sign-in token is still the sign-in session");

    let mut held_by_an_application = claims_of(&own);
    held_by_an_application.client_id = Some(APPLICATION.to_owned());
    let refused = auth()
        .validate_authorization_session_token(&signed(&held_by_an_application), &JWKS)
        .expect_err("a token an application holds is not the sign-in session");
    assert_eq!(
        refused.code,
        ErrorCode::AuthInvalid,
        "refused as no session, so the screen sends its holder to log in"
    );

    let mut narrow = claims_of(&own);
    narrow.scope = OAuthScope::render_granted(&OAuthScope::delegable_grant());
    assert_eq!(
        narrow.aud, OAUTH2_AUTHORIZE,
        "still addressed to the authorization server, so the refusal is for the grant"
    );
    auth()
        .validate_authorization_session_token(&signed(&narrow), &JWKS)
        .expect_err("a narrow grant is not the sign-in session");
}
