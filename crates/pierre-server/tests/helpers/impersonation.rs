// ABOUTME: An admin's support session on the athlete an ordinary session token names, for the suites that pin what it may not do
// ABOUTME: Mints a live impersonation token on the same user and active tenant, opened by a fresh admin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Every binary that loads `helpers` compiles this; only some suites call it.
#![allow(dead_code)]

use std::sync::Arc;

use pierre_auth::auth::{
    ImpersonationGrant, IMPERSONATION_REFUSED_ACTION, IMPERSONATION_WRITE_REFUSED,
};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_database::repositories::admin_audit::AuditEventFilter;
use pierre_mcp_server::mcp::resources::ServerContext;
use uuid::Uuid;

use crate::common::create_test_user_with_email;
use dravr_tronc::dates::{Elapsed, Timestamp};

/// A live support session on `target`, opened by a fresh admin, as a chat
/// turn typed in it carries it ([`pierre_tool_runtime::context::SUPPORT_SESSION`]).
pub(crate) async fn support_grant(
    resources: &Arc<ServerContext>,
    target: Uuid,
) -> ImpersonationGrant {
    let admin_id = fresh_admin(resources).await;
    let session = ImpersonationSession::new(admin_id, target, Some("support".to_owned()));
    resources
        .common
        .repos
        .impersonation
        .create_session(&session)
        .await
        .unwrap();
    ImpersonationGrant {
        session_id: session.id,
        impersonator_id: admin_id,
        expires_at: (Timestamp::now() + Elapsed::hours(1)).epoch_seconds(),
    }
}

/// A fresh admin account.
async fn fresh_admin(resources: &Arc<ServerContext>) -> Uuid {
    let (admin_id, _) = create_test_user_with_email(
        &resources.agent.database,
        &format!("support-{}@example.com", Uuid::new_v4()),
    )
    .await
    .unwrap();
    resources
        .common
        .repos
        .users
        .set_admin_status(admin_id, true)
        .await
        .unwrap();
    admin_id
}

/// A `Bearer` header for a live impersonation of the athlete that
/// `session_bearer` (an ordinary `Bearer <jwt>` session) acts as, in the same
/// active tenant, opened by a fresh admin.
///
/// The two headers differ only in the impersonation, so a request one is
/// refused and the other served shows the refusal is the impersonation's.
pub(crate) async fn impersonation_of(
    resources: &Arc<ServerContext>,
    session_bearer: &str,
) -> String {
    let auth = &resources.auth;
    let claims = auth
        .auth_manager
        .validate_token(
            session_bearer.trim_start_matches("Bearer "),
            &auth.jwks_manager,
        )
        .expect("an ordinary session token");
    let target_id = Uuid::parse_str(&claims.sub).unwrap();
    // SECURITY: Global lookup — a test fixture reading the user its token names
    let target = resources
        .common
        .repos
        .users
        .get_global(target_id)
        .await
        .unwrap()
        .expect("the session's user");

    let admin_id = fresh_admin(resources).await;

    let session = ImpersonationSession::new(admin_id, target.id, Some("support".to_owned()));
    resources
        .common
        .repos
        .impersonation
        .create_session(&session)
        .await
        .unwrap();
    let token = auth
        .auth_manager
        .generate_impersonation_token(
            &target,
            admin_id,
            &session.id,
            &auth.jwks_manager,
            claims.active_tenant_id,
        )
        .unwrap();
    format!("Bearer {token}")
}

/// Assert a response is the refusal a support session gets for a write that
/// would outlive it: a `403` naming who can do it instead, so the admin stays
/// signed in (a `401` signs the web and mobile clients out).
pub(crate) fn assert_support_refused(status: u16, body: &str, what: &str) {
    assert_eq!(status, 403, "a support session cannot {what}: {body}");
    assert!(
        body.contains(IMPERSONATION_WRITE_REFUSED),
        "a support session cannot {what}, and the refusal says so: {body}"
    );
}

/// Assert the admin audit trail holds the refusal of `attempted` on `target`'s
/// account, attributed to the admin who made it.
pub(crate) async fn assert_refusal_audited(
    resources: &Arc<ServerContext>,
    target: Uuid,
    attempted: &str,
) {
    let filter = AuditEventFilter {
        action_prefix: Some(IMPERSONATION_REFUSED_ACTION.to_owned()),
        ..AuditEventFilter::default()
    };
    let rows = resources
        .common
        .repos
        .admin_audit
        .list(&filter, 200)
        .await
        .unwrap();
    let row = rows
        .iter()
        .find(|row| {
            row.metadata["target"]["user_id"] == target.to_string()
                && row.metadata["attempted"] == attempted
        })
        .unwrap_or_else(|| panic!("no audit row refusing {attempted:?} on {target}: {rows:?}"));
    assert_eq!(
        row.outcome, "refused",
        "the audit row records a refusal: {row:?}"
    );
    assert!(
        row.actor_user_id.is_some(),
        "the refusal names the admin who attempted it"
    );
    assert!(
        row.metadata["impersonation_session_id"].is_string(),
        "the refusal names the support session"
    );
}
