// ABOUTME: What a Firebase (Google) sign-in may do with the email its ID token carries: verify it, attach to it, or neither
// ABOUTME: Pins that only a verified claim naming the account's email marks it verified or attaches to an existing account
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Google verified the address a Firebase ID token carries when the token's
//! `email_verified` claim is true. Such a token proves the address, so a
//! sign-in through `POST /api/auth/firebase`:
//!
//! - stamps the account's verified mark, as the confirmation link does, and
//!   the login answers `email_verified: true`;
//! - may attach the Firebase user to an existing account holding that email.
//!
//! A token whose claim is false or absent proves nothing. It still starts a
//! new account for an email no account holds, unverified, as every new sign-up
//! does. It never reaches an existing account by email: that sign-in is
//! refused with the generic credentials refusal, which does not say the
//! account exists, and leaves no account holding the Firebase user. A verified
//! claim marks an account only when the address it names is the account's own
//! email, case aside, and an email in another casing than the account's
//! reaches that account rather than opening a second one.
//!
//! A test signer plays Google's `securetoken` key set (see
//! `helpers::google_token`), so the tokens are validated exactly as Firebase's
//! are, against a key served from a local listener.

mod common;
mod helpers;

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, create_test_user_with_plan};
use dravr_tronc::iam::GoogleKeySet;
use helpers::axum_test::AxumTestRequest;
use helpers::google_token::{now_secs, TestSigner};
use pierre_auth::firebase::FirebaseAuth;
use pierre_config::environment::FirebaseConfig;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

const PROJECT: &str = "dravr-test-project";

/// What any refused credential reads as on the wire: the generic description
/// of an invalid credential, naming no account.
const GENERIC_REFUSAL: &str = "The provided authentication credentials are invalid";

/// The claims of a Firebase ID token for a Google sign-in, as Firebase mints
/// them.
#[derive(Debug, Clone, Serialize)]
struct FirebaseTokenClaims {
    iss: String,
    aud: String,
    sub: String,
    iat: u64,
    exp: u64,
    email: Option<String>,
    email_verified: Option<bool>,
    firebase: HashMap<String, Value>,
}

/// A Google sign-in to `PROJECT` by the Firebase user `uid` with `email`,
/// valid for an hour, whose address Google verified when `verified`.
fn google_sign_in(uid: &str, email: &str, verified: Option<bool>) -> FirebaseTokenClaims {
    let now = now_secs();
    FirebaseTokenClaims {
        iss: format!("https://securetoken.google.com/{PROJECT}"),
        aud: PROJECT.to_owned(),
        sub: uid.to_owned(),
        iat: now,
        exp: now + 3600,
        email: Some(email.to_owned()),
        email_verified: verified,
        firebase: HashMap::from([("sign_in_provider".to_owned(), json!("google.com"))]),
    }
}

struct World {
    res: Arc<ServerContext>,
    router: Router,
    signer: TestSigner,
}

impl World {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let signer = TestSigner::generate();
        let jwks = signer.serve_jwks().await;
        let mut ctx = res.auth_routes_context();
        ctx.firebase_auth = Some(Arc::new(FirebaseAuth::with_key_set(
            FirebaseConfig {
                project_id: Some(PROJECT.to_owned()),
                api_key: None,
                enabled: true,
            },
            GoogleKeySet::new(&jwks.url, reqwest::Client::new()),
        )));
        Self {
            res,
            router: AuthRoutes::routes(ctx),
            signer,
        }
    }

    /// Sign in through Firebase with `claims`: the status and the body.
    async fn attempt(&self, claims: &FirebaseTokenClaims) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post("/api/auth/firebase")
            .json(&json!({ "id_token": self.signer.mint(claims) }))
            .send(self.router.clone())
            .await;
        let status = resp.status_code();
        (
            status,
            serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
        )
    }

    /// Sign in through Firebase with `claims`; the login's user.
    async fn sign_in(&self, claims: &FirebaseTokenClaims) -> Value {
        let (status, body) = self.attempt(claims).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["user"].clone()
    }

    async fn verified(&self, user_id: Uuid) -> bool {
        self.res
            .common
            .repos
            .email_verification
            .is_verified(user_id)
            .await
            .unwrap()
    }

    async fn user_by_email(&self, email: &str) -> Uuid {
        self.res
            .common
            .repos
            .users
            .get_by_email(email)
            .await
            .unwrap()
            .expect("the sign-in left an account")
            .id
    }

    /// The account holding the Firebase user `uid`, if any does.
    async fn holder_of(&self, uid: &str) -> Option<Uuid> {
        self.res
            .common
            .repos
            .users
            .get_by_firebase_uid(uid)
            .await
            .unwrap()
            .map(|user| user.id)
    }

    /// A password account for `email`, never verified.
    async fn password_account(&self, email: &str) -> Uuid {
        let (account, _, _) =
            create_test_user_with_plan(&self.res.agent.database, email, "professional")
                .await
                .unwrap();
        assert!(!self.verified(account).await);
        account
    }
}

/// A new account made by a Google sign-in whose token says Google verified
/// the address starts verified.
async fn a_verified_google_sign_up_is_verified(w: &World) {
    let user = w
        .sign_in(&google_sign_in(
            "uid-verified",
            "verified@firebase.test",
            Some(true),
        ))
        .await;
    assert_eq!(user["email_verified"], true, "{user}");
    let account = w.user_by_email("verified@firebase.test").await;
    assert!(w.verified(account).await);
    assert_eq!(w.holder_of("uid-verified").await, Some(account));
}

/// A token whose claim is false, or carries none, still starts a new account
/// for an email no account holds, and marks nothing.
async fn an_unverified_sign_up_starts_an_unverified_account(w: &World) {
    let user = w
        .sign_in(&google_sign_in(
            "uid-unverified",
            "unverified@firebase.test",
            Some(false),
        ))
        .await;
    assert_eq!(user["email_verified"], false, "{user}");
    let account = w.user_by_email("unverified@firebase.test").await;
    assert!(!w.verified(account).await);
    assert_eq!(w.holder_of("uid-unverified").await, Some(account));

    let user = w
        .sign_in(&google_sign_in("uid-silent", "silent@firebase.test", None))
        .await;
    assert_eq!(user["email_verified"], false, "{user}");
    let account = w.user_by_email("silent@firebase.test").await;
    assert!(!w.verified(account).await);
}

/// A token that does not prove its email never reaches the existing account
/// that email names: the sign-in is refused as any bad credential is, and no
/// account holds the Firebase user afterwards.
async fn an_unverified_email_never_reaches_an_existing_account(w: &World) {
    let account = w.password_account("victim@firebase.test").await;

    for claim in [Some(false), None] {
        let (status, body) = w
            .attempt(&google_sign_in(
                "uid-intruder",
                "victim@firebase.test",
                claim,
            ))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{claim:?}: {body}");
        assert_eq!(body["message"], GENERIC_REFUSAL, "{claim:?}: {body}");
        assert!(body.get("jwt_token").is_none(), "{body}");
        let text = body.to_string();
        assert!(
            !text.contains("victim"),
            "the refusal names no account: {text}"
        );
        assert!(!text.contains(&account.to_string()), "{text}");
    }

    assert_eq!(w.holder_of("uid-intruder").await, None, "nothing attached");
    let stored = w
        .res
        .common
        .repos
        .users
        .get_global(account)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.firebase_uid, None);
    assert!(!w.verified(account).await);
    assert_eq!(w.user_by_email("victim@firebase.test").await, account);
}

/// A token that proves the account's own email attaches the Firebase user to
/// it, and marks it verified.
async fn a_verified_email_attaches_to_the_existing_account(w: &World) {
    let account = w.password_account("runner@firebase.test").await;

    let user = w
        .sign_in(&google_sign_in(
            "uid-runner",
            "runner@firebase.test",
            Some(true),
        ))
        .await;
    assert_eq!(user["user_id"], account.to_string(), "{user}");
    assert_eq!(user["email_verified"], true, "{user}");
    assert_eq!(w.holder_of("uid-runner").await, Some(account));
    assert!(w.verified(account).await);
}

/// Emails are case-insensitive: a verified Google sign-in whose provider
/// reports the address in another casing, or with stray whitespace, is the
/// existing verified account's owner. It reaches that account rather than
/// opening a second one.
async fn a_verified_email_in_another_casing_reaches_the_existing_account(w: &World) {
    let account = w.password_account("mixed@firebase.test").await;
    w.res
        .common
        .repos
        .email_verification
        .mark_verified(account)
        .await
        .unwrap();

    let user = w
        .sign_in(&google_sign_in(
            "uid-mixed",
            " Mixed@Firebase.TEST",
            Some(true),
        ))
        .await;
    assert_eq!(user["user_id"], account.to_string(), "{user}");
    assert_eq!(user["email"], "mixed@firebase.test", "{user}");
    assert_eq!(user["email_verified"], true, "{user}");
    assert_eq!(w.holder_of("uid-mixed").await, Some(account));
    assert_eq!(w.user_by_email("MIXED@firebase.test").await, account);
}

/// An account the Firebase user already holds signs in by its UID. A verified
/// claim for another address (their Google address moved) vouches for none of
/// the account's; one for the account's own email, case aside, marks it.
async fn a_held_account_is_marked_only_by_its_own_verified_email(w: &World) {
    let account = w.password_account("swimmer@firebase.test").await;
    let repos = &w.res.common.repos;
    let mut held = repos.users.get_global(account).await.unwrap().unwrap();
    held.firebase_uid = Some("uid-swimmer".to_owned());
    "google.com".clone_into(&mut held.auth_provider);
    repos.users.update(&held).await.unwrap();

    let user = w
        .sign_in(&google_sign_in(
            "uid-swimmer",
            "swimmer-new@firebase.test",
            Some(true),
        ))
        .await;
    assert_eq!(user["user_id"], account.to_string(), "signs in by its UID");
    assert!(!w.verified(account).await, "another address proves nothing");

    let user = w
        .sign_in(&google_sign_in(
            "uid-swimmer",
            "Swimmer@Firebase.Test",
            Some(true),
        ))
        .await;
    assert_eq!(user["user_id"], account.to_string());
    assert_eq!(user["email_verified"], true, "{user}");
    assert!(w.verified(account).await);
}

#[tokio::test]
async fn a_firebase_sign_in_trusts_its_email_only_when_the_token_proves_it() {
    let w = World::new().await;

    a_verified_google_sign_up_is_verified(&w).await;
    an_unverified_sign_up_starts_an_unverified_account(&w).await;
    an_unverified_email_never_reaches_an_existing_account(&w).await;
    a_verified_email_attaches_to_the_existing_account(&w).await;
    a_verified_email_in_another_casing_reaches_the_existing_account(&w).await;
    a_held_account_is_marked_only_by_its_own_verified_email(&w).await;
}
