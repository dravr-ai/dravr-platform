// ABOUTME: A consent grant's receipts: one event and actor per caller path, one ending per grant, expiry, baseline, outbox, routes
// ABOUTME: Runs on whichever backend DATABASE_URL names; the worker drains into an in-memory barrage log with a local signer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Owl phase 5 slice 3b, item 2 and 3 (carnet#1544, design row 34).
//!
//! Every statement that starts or ends an `oauth_client_grants` row writes
//! the grant's receipt in its transaction. These tests drive each caller
//! path and read the receipts back: the event, the actor, the time, and the
//! payload the receipt states. A grant ends once: a revoke and the expire
//! sweep racing write one ending, and a grant past its expiry ends as
//! `expire` whoever ends it. The `record_log` worker baselines the grants
//! given before receipts, appends each receipt's SHA-256 to the athlete's
//! log once, and signs it as a JWS that verifies under the checkpoint key.

use crate::common;
use crate::helpers::axum_test::AxumTestRequest;

use std::sync::Arc;

use axum::http::StatusCode;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use dravr_barrage::digest::Sha256 as BarrageSha256;
use dravr_barrage::keyed::Signer;
use dravr_barrage::record_log::entry::EntryKind;
use dravr_barrage::record_log::{all_entries, grant_key, RecordLog};
use dravr_barrage::source::SourceKey;
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::Scope;
use dravr_tronc::dates::{Elapsed, SecondsFormat, Timestamp};
use pierre_core::models::{
    GrantActor, GrantApproval, GrantEvent, OAuthClientGrant, OAuthGrantEvent, User,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::MAX_GRANT_RECEIPT_ATTEMPTS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::oauth_grants::OAuthGrantsRoutes;
use pierre_services::auth::{FederatedIdentity, SignupSource};
use pierre_services::barrage_keys::{checkpoint_key_id, memory_records, LocalSigner};
use pierre_services::record_log_worker::{GrantReceiptOutbox, RecordLogWorker};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SCOPE: &str = "fitness:read";
const OTHER_SCOPE: &str = "fitness:read profile:read";
const ISSUER: &str = "https://api.dravr.test";
/// A pre-3b grant: its receipts never written.
const FORGET_RECEIPTS_SQL: &str = "DELETE FROM oauth_grant_events WHERE client_grant_id = $1";

/// An athlete in their own tenant.
struct Athlete {
    resources: Arc<ServerContext>,
    user: User,
    user_id: String,
    tenant_id: String,
}

async fn athlete() -> Athlete {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("receipts_{}@example.com", Uuid::new_v4());
    let (user_id, user) = common::create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|tenant| tenant.owner_user_id == user_id)
        .expect("a fresh user owns a tenant")
        .id
        .to_string();
    Athlete {
        resources,
        user,
        user_id: user_id.to_string(),
        tenant_id,
    }
}

impl Athlete {
    /// A grant to `client_id` for `scope`, expiring at `expires_at`.
    fn grant(
        &self,
        client_id: &str,
        scope: &str,
        expires_at: Option<Timestamp>,
    ) -> OAuthClientGrant {
        OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: self.user_id.clone(),
            tenant_id: self.tenant_id.clone(),
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: Some("digest-1".to_owned()),
            consent_locale: Some("en".to_owned()),
            authorization_details: None,
            expires_at,
        }
    }

    /// Store `grant` as the consent screen does: approved by the athlete.
    async fn approve(&self, grant: &OAuthClientGrant) -> String {
        self.resources
            .common
            .repos
            .oauth2_server
            .store_client_grant(grant, &GrantApproval::by_user())
            .await
            .unwrap()
    }

    /// The receipts of `grant_id`, newest first.
    async fn receipts(&self, grant_id: &str) -> Vec<OAuthGrantEvent> {
        self.resources
            .common
            .repos
            .oauth_grant_events
            .list_for_user(&self.tenant_id, &self.user_id, Some(grant_id), 100, 0)
            .await
            .unwrap()
    }

    /// The one ending receipt of `grant_id`.
    async fn ending(&self, grant_id: &str) -> OAuthGrantEvent {
        let endings: Vec<_> = self
            .receipts(grant_id)
            .await
            .into_iter()
            .filter(|receipt| receipt.event != GrantEvent::Issue)
            .collect();
        assert_eq!(
            endings.len(),
            1,
            "bug: a grant ended {} times",
            endings.len()
        );
        endings.into_iter().next().unwrap()
    }

    /// The one `issue` receipt of `grant_id`.
    async fn issue(&self, grant_id: &str) -> OAuthGrantEvent {
        let issues: Vec<_> = self
            .receipts(grant_id)
            .await
            .into_iter()
            .filter(|receipt| receipt.event == GrantEvent::Issue)
            .collect();
        assert_eq!(issues.len(), 1, "bug: {} issue receipts", issues.len());
        issues.into_iter().next().unwrap()
    }

    /// The stored grant row, live or revoked.
    async fn stored(&self, grant_id: &str) -> Option<OAuthClientGrant> {
        self.resources
            .common
            .repos
            .oauth2_server
            .get_client_grant(grant_id, &self.user_id, &self.tenant_id)
            .await
            .unwrap()
    }

    /// Run the client sweeper's pass now, reclaiming no registration.
    async fn sweep(&self) -> u64 {
        let long_ago = Timestamp::now() - Elapsed::days(10_000);
        self.resources
            .common
            .repos
            .oauth2_server
            .delete_stale_clients(long_ago, long_ago, Timestamp::now())
            .await
            .unwrap()
            .expired_grants
    }

    /// Forget `grant_id`'s receipts, as a grant given before 3b has none.
    async fn forget_receipts(&self, grant_id: &str) {
        match self.resources.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(FORGET_RECEIPTS_SQL)
                    .bind(grant_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(FORGET_RECEIPTS_SQL)
                    .bind(grant_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    fn scope(&self) -> Scope {
        Scope::new(
            Uuid::parse_str(&self.tenant_id).unwrap(),
            Uuid::parse_str(&self.user_id).unwrap(),
        )
    }

    fn worker(
        &self,
        records: Arc<dyn ActivityRecordStore>,
        signer: Option<Arc<dyn Signer>>,
    ) -> RecordLogWorker {
        let repos = &self.resources.common.repos;
        RecordLogWorker::new(
            records,
            Arc::clone(&repos.import_tombstones),
            Arc::clone(&repos.account_deletion),
            Arc::clone(&repos.users),
            GrantReceiptOutbox {
                events: Arc::clone(&repos.oauth_grant_events),
                issuer_url: ISSUER.to_owned(),
            },
            signer,
            None,
        )
    }
}

/// A time as a receipt's payload states it: RFC 3339, microseconds, `Z`.
fn receipt_time(at: Timestamp) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn client_id() -> String {
    format!("https://client-{}.example/meta.json", Uuid::new_v4())
}

fn payload(receipt: &OAuthGrantEvent) -> Value {
    serde_json::from_str(&receipt.payload).unwrap()
}

fn sha256_hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[tokio::test]
async fn an_approval_writes_its_issue_receipt_stating_the_stored_grant() {
    let athlete = athlete().await;
    let grant = athlete.grant(&client_id(), SCOPE, None);
    let stored = athlete.approve(&grant).await;
    let row = athlete.stored(&stored).await.unwrap();

    let issue = athlete.issue(&stored).await;
    assert_eq!(
        issue.actor,
        GrantActor::User,
        "bug: the athlete's approval is not theirs"
    );
    assert_eq!(
        issue.at, row.granted_at,
        "bug: the issue is not at the row's granted_at"
    );
    assert_eq!(
        issue.payload_sha256,
        sha256_hex(&issue.payload),
        "bug: the stored digest is not of the stored bytes"
    );
    assert!(
        issue.jws.is_none() && issue.logged_at.is_none(),
        "bug: born drained"
    );
    let body = payload(&issue);
    assert_eq!(body["receipt_id"], issue.id.as_str(), "bug: receipt_id");
    assert_eq!(body["event"], "issue", "bug: event");
    assert_eq!(body["at"], receipt_time(row.granted_at).as_str(), "bug: at");
    assert_eq!(body["grant"]["id"], stored.as_str(), "bug: grant id");
    assert_eq!(
        body["grant"]["subject"],
        format!("urn:dravr:user:{}", athlete.user_id).as_str(),
        "bug: subject"
    );
    assert_eq!(body["grant"]["scope"], SCOPE, "bug: scope");
    assert_eq!(
        body["consent_evidence"]["review_digest"], "digest-1",
        "bug: digest"
    );
    assert_eq!(body["approval"]["actor"], "user", "bug: actor");
    assert!(
        body["approval"]["standing_policy_ref"].is_null(),
        "bug: policy"
    );
}

#[tokio::test]
async fn a_re_consent_in_place_issues_again_with_the_new_digest() {
    let athlete = athlete().await;
    let client = client_id();
    let first = athlete.approve(&athlete.grant(&client, SCOPE, None)).await;
    let mut again = athlete.grant(&client, SCOPE, None);
    again.consent_version = Some("digest-2".to_owned());
    let held = athlete.approve(&again).await;
    assert_eq!(held, first, "premise: a re-consent keeps its row");

    let receipts = athlete.receipts(&first).await;
    assert_eq!(receipts.len(), 2, "bug: a re-consent wrote no second issue");
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.event == GrantEvent::Issue),
        "bug: a re-consent ended its grant"
    );
    let reconsent = receipts
        .iter()
        .map(payload)
        .find(|body| body["consent_evidence"]["review_digest"] == "digest-2")
        .expect("bug: no receipt states the re-consent's digest");
    assert_eq!(
        reconsent["grant"]["id"],
        first.as_str(),
        "bug: the receipt states the request, not the stored grant"
    );
}

#[tokio::test]
async fn an_approval_revokes_the_clients_other_grant_as_the_athlete() {
    let athlete = athlete().await;
    let client = client_id();
    let older = athlete.approve(&athlete.grant(&client, SCOPE, None)).await;
    let older_issue = athlete.issue(&older).await;
    let newer = athlete
        .approve(&athlete.grant(&client, OTHER_SCOPE, None))
        .await;

    let ending = athlete.ending(&older).await;
    assert_eq!(
        ending.event,
        GrantEvent::Revoke,
        "bug: the other grant did not end as revoked"
    );
    assert_eq!(
        ending.actor,
        GrantActor::User,
        "bug: the approval's revoke is not the athlete's"
    );
    let row = athlete.stored(&older).await.unwrap();
    assert_eq!(
        Some(ending.at),
        row.revoked_at,
        "bug: the revoke is not at revoked_at"
    );
    assert_eq!(
        payload(&ending)["issue_receipt_id"],
        older_issue.id.as_str(),
        "bug: the revoke does not name the grant's issue"
    );
    athlete.issue(&newer).await;
}

#[tokio::test]
async fn a_replacement_revokes_the_live_grant_and_issues_the_new() {
    let athlete = athlete().await;
    let client = client_id();
    let live = athlete.approve(&athlete.grant(&client, SCOPE, None)).await;
    let replacement = athlete.grant(&client, SCOPE, None);
    let stored = athlete
        .resources
        .common
        .repos
        .oauth2_server
        .replace_client_grant(&live, &replacement, &GrantApproval::by_user())
        .await
        .unwrap();

    let ending = athlete.ending(&live).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Revoke, GrantActor::User),
        "bug: the replaced grant's ending"
    );
    assert_eq!(
        athlete.issue(&stored).await.actor,
        GrantActor::User,
        "bug: new issue"
    );
}

#[tokio::test]
async fn revoking_in_connected_apps_writes_the_athletes_revoke() {
    let athlete = athlete().await;
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    let session = common::generate_test_token(&athlete.resources, &athlete.user).await;
    AxumTestRequest::delete(&format!("/api/me/oauth-grants/{grant}"))
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&athlete.resources)))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let ending = athlete.ending(&grant).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Revoke, GrantActor::User),
        "bug: Connected apps' revoke"
    );
}

#[tokio::test]
async fn a_federated_retirement_revokes_as_the_as() {
    let athlete = athlete().await;
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    // A first Google sign-in proving an address the account never verified
    // retires every credential issued before it, the grant included.
    athlete
        .resources
        .account_service()
        .login_with_federated_identity(FederatedIdentity {
            firebase_uid: Some("receipts-firebase-uid"),
            google_subject: Some("108000000000000000099"),
            email: athlete.user.email.clone(),
            email_verified: true,
            display_name: None,
            provider: "google.com",
            signup_source: SignupSource::Firebase,
            locale: None,
        })
        .await
        .unwrap();

    let ending = athlete.ending(&grant).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Revoke, GrantActor::Auto),
        "bug: a federated retirement's ending"
    );
}

#[tokio::test]
async fn a_client_withdrawal_revokes_each_live_grant_as_the_client() {
    let athlete = athlete().await;
    let client = client_id();
    let grant = athlete.approve(&athlete.grant(&client, SCOPE, None)).await;
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .withdraw_client_access(&athlete.user_id, &athlete.tenant_id, &client, SCOPE)
        .await
        .unwrap();

    let ending = athlete.ending(&grant).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Revoke, GrantActor::Client),
        "bug: an RFC 7009 withdrawal's ending"
    );
}

#[tokio::test]
async fn a_withdrawal_without_a_grant_row_records_one_born_revoked_with_its_receipt() {
    let athlete = athlete().await;
    let client = client_id();
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .withdraw_client_access(&athlete.user_id, &athlete.tenant_id, &client, SCOPE)
        .await
        .unwrap();

    let receipts = athlete
        .resources
        .common
        .repos
        .oauth_grant_events
        .list_for_user(&athlete.tenant_id, &athlete.user_id, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(
        receipts.len(),
        1,
        "bug: the born-revoked row has no receipt"
    );
    let receipt = &receipts[0];
    assert_eq!(
        (receipt.event, receipt.actor),
        (GrantEvent::Revoke, GrantActor::Client),
        "bug: the withdrawal's receipt"
    );
    let body = payload(receipt);
    assert!(
        body.get("issue_receipt_id").is_some_and(Value::is_null),
        "bug: a grant never issued names an issue: {body}"
    );
    assert_eq!(body["grant"]["client_id"], client.as_str(), "bug: client");
}

#[tokio::test]
async fn deleting_a_client_ends_its_live_grants_as_the_client_and_keeps_the_receipts() {
    let athlete = athlete().await;
    let client = client_id();
    let grant = athlete.approve(&athlete.grant(&client, SCOPE, None)).await;
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .delete_client(&client)
        .await
        .unwrap();

    assert!(
        athlete.stored(&grant).await.is_none(),
        "premise: the row is deleted"
    );
    let ending = athlete.ending(&grant).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Revoke, GrantActor::Client),
        "bug: an A2A client's deletion"
    );
    assert_eq!(
        athlete.receipts(&grant).await.len(),
        2,
        "bug: the receipts went with the row"
    );
}

#[tokio::test]
async fn the_orphan_sweep_ends_live_grants_as_the_as() {
    let athlete = athlete().await;
    // A grant naming a client that is not registered: what a deleted
    // registration leaves behind.
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    athlete.sweep().await;

    assert!(
        athlete.stored(&grant).await.is_none(),
        "premise: the orphan is deleted"
    );
    let ending = athlete.ending(&grant).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Revoke, GrantActor::Auto),
        "bug: the orphan sweep's ending"
    );
}

#[tokio::test]
async fn a_grant_past_its_expiry_ends_as_expired_whoever_ends_it() {
    let athlete = athlete().await;
    let repos = &athlete.resources.common.repos;
    let expired = Timestamp::now() - Elapsed::minutes(5);

    // The athlete revokes it in Connected apps.
    let revoked = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, Some(expired)))
        .await;
    repos
        .oauth2_server
        .revoke_client_grant(
            &revoked,
            &athlete.user_id,
            &athlete.tenant_id,
            GrantActor::User,
        )
        .await
        .unwrap();
    // The client withdraws it.
    let withdrawn_client = client_id();
    let withdrawn = athlete
        .approve(&athlete.grant(&withdrawn_client, SCOPE, Some(expired)))
        .await;
    repos
        .oauth2_server
        .withdraw_client_access(
            &athlete.user_id,
            &athlete.tenant_id,
            &withdrawn_client,
            SCOPE,
        )
        .await
        .unwrap();
    // Its client is deleted.
    let deleted_client = client_id();
    let deleted = athlete
        .approve(&athlete.grant(&deleted_client, SCOPE, Some(expired)))
        .await;
    repos
        .oauth2_server
        .delete_client(&deleted_client)
        .await
        .unwrap();

    for grant in [&revoked, &withdrawn, &deleted] {
        let ending = athlete.ending(grant).await;
        assert_eq!(
            (ending.event, ending.actor),
            (GrantEvent::Expire, GrantActor::Auto),
            "bug: a grant past its expiry did not end as expired"
        );
        assert_eq!(
            receipt_time(ending.at),
            receipt_time(expired),
            "bug: the expiry is not at expires_at"
        );
    }
}

#[tokio::test]
async fn the_sweep_expires_a_grant_once_and_leaves_its_row_listed() {
    let athlete = athlete().await;
    let expired = Timestamp::now() - Elapsed::minutes(5);
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, Some(expired)))
        .await;
    // Registered, so the orphan delete keeps the row.
    let repos = &athlete.resources.common.repos;
    let client = athlete.stored(&grant).await.unwrap().client_id;
    register(&athlete, &client).await;

    assert_eq!(athlete.sweep().await, 1, "bug: the sweep expired no grant");
    assert_eq!(
        athlete.sweep().await,
        0,
        "bug: a second sweep expired it again"
    );
    let ending = athlete.ending(&grant).await;
    assert_eq!(
        (ending.event, ending.actor),
        (GrantEvent::Expire, GrantActor::Auto),
        "bug: the sweep's ending"
    );
    let row = athlete.stored(&grant).await.unwrap();
    assert!(row.revoked_at.is_none(), "bug: the sweep revoked the row");

    // The athlete revokes it afterwards: the row is revoked, no second
    // ending is written.
    assert!(
        repos
            .oauth2_server
            .revoke_client_grant(
                &grant,
                &athlete.user_id,
                &athlete.tenant_id,
                GrantActor::User
            )
            .await
            .unwrap(),
        "bug: the athlete could not revoke a grant the sweep only expired"
    );
    athlete.ending(&grant).await;
}

#[tokio::test]
async fn a_revoke_and_the_sweep_racing_write_one_ending() {
    let athlete = athlete().await;
    let expired = Timestamp::now() - Elapsed::minutes(5);
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, Some(expired)))
        .await;
    let client = athlete.stored(&grant).await.unwrap().client_id;
    register(&athlete, &client).await;
    let repos = &athlete.resources.common.repos;

    let (revoked, _) = tokio::join!(
        repos.oauth2_server.revoke_client_grant(
            &grant,
            &athlete.user_id,
            &athlete.tenant_id,
            GrantActor::User
        ),
        athlete.sweep(),
    );
    assert!(revoked.unwrap(), "premise: the revoke went through");
    let ending = athlete.ending(&grant).await;
    assert_eq!(ending.event, GrantEvent::Expire, "bug: the race's ending");
}

#[tokio::test]
async fn a_re_approval_inside_the_sweeps_interval_ends_the_old_grant_as_expired() {
    let athlete = athlete().await;
    let client = client_id();
    let expired = Timestamp::now() - Elapsed::minutes(5);
    let old = athlete
        .approve(&athlete.grant(&client, SCOPE, Some(expired)))
        .await;
    // The consent submit replaces a live row past its expiry.
    let stored = athlete
        .resources
        .common
        .repos
        .oauth2_server
        .replace_client_grant(
            &old,
            &athlete.grant(&client, SCOPE, None),
            &GrantApproval::by_user(),
        )
        .await
        .unwrap();

    let ending = athlete.ending(&old).await;
    assert_eq!(
        (ending.event, ending.actor, receipt_time(ending.at)),
        (GrantEvent::Expire, GrantActor::Auto, receipt_time(expired)),
        "bug: an expired grant re-approved did not end as expired"
    );
    athlete.issue(&stored).await;
}

#[tokio::test]
async fn the_baseline_gives_a_grant_given_before_receipts_one_issue() {
    let athlete = athlete().await;
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    athlete.forget_receipts(&grant).await;
    let events = &athlete.resources.common.repos.oauth_grant_events;

    assert_eq!(
        events.baseline_live_grants(500).await.unwrap(),
        1,
        "bug: no baseline"
    );
    assert_eq!(
        events.baseline_live_grants(500).await.unwrap(),
        0,
        "bug: baselined twice"
    );
    let issue = athlete.issue(&grant).await;
    let row = athlete.stored(&grant).await.unwrap();
    assert_eq!(
        issue.actor,
        GrantActor::Auto,
        "bug: a baseline is the athlete's"
    );
    assert_eq!(
        issue.at, row.granted_at,
        "bug: a baseline is not at granted_at"
    );
    assert_eq!(
        payload(&issue)["approval"]["baseline"],
        true,
        "bug: not marked a baseline"
    );
}

#[tokio::test]
async fn the_worker_baselines_and_logs_each_receipt_once() {
    let athlete = athlete().await;
    let records = Arc::new(memory_records());
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    // A grant given before receipts: the worker baselines it.
    let before = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    athlete.forget_receipts(&before).await;

    let tick = athlete.worker(records.clone(), None).run().await.unwrap();
    assert_eq!(
        (
            tick.grant_receipts_baselined,
            tick.grant_receipts_logged,
            tick.grant_receipts_signed
        ),
        (1, 2, 0),
        "bug: one baseline, two receipts logged, none signed without a signer"
    );
    let issue = athlete.issue(&grant).await;
    assert!(issue.logged_at.is_some(), "bug: not marked logged");
    assert!(issue.jws.is_none(), "bug: a JWS without a signer");

    // The entry the athlete's export carries names the receipt's SHA-256.
    let scope = athlete.scope();
    let key = grant_key(records.keyed(), &scope, &grant).unwrap();
    let logged: Vec<_> = all_entries(records.as_ref(), &scope)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.record_key.as_ref() == Some(&key))
        .collect();
    assert_eq!(logged.len(), 1, "bug: the grant's issue is not logged once");
    assert_eq!(
        (logged[0].kind, logged[0].source),
        (EntryKind::GrantIssue, SourceKey::OAuthGrant),
        "bug: the entry is not the grant's issue"
    );
    assert_eq!(
        logged[0].content.map(|content| content.to_hex()),
        Some(issue.payload_sha256),
        "bug: the entry does not name the receipt's SHA-256"
    );
}

#[tokio::test]
async fn a_retried_drain_appends_nothing_and_a_signer_signs_every_receipt() {
    let athlete = athlete().await;
    let records = Arc::new(memory_records());
    let grant = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .revoke_client_grant(
            &grant,
            &athlete.user_id,
            &athlete.tenant_id,
            GrantActor::User,
        )
        .await
        .unwrap();
    // A drain that appended the ending before it could mark the row.
    let scope = athlete.scope();
    let ending = athlete.ending(&grant).await;
    let content = BarrageSha256::parse_hex(&ending.payload_sha256).unwrap();
    assert!(
        records
            .log_grant(&scope, EntryKind::GrantRevoke, &grant, content)
            .await
            .unwrap(),
        "fixture: the crashed drain's entry was not appended"
    );

    let signer = Arc::new(LocalSigner::random().unwrap());
    let tick = athlete
        .worker(records.clone(), Some(signer.clone()))
        .run()
        .await
        .unwrap();
    assert_eq!(
        (tick.grant_receipts_logged, tick.grant_receipts_signed),
        (2, 2),
        "bug: both receipts are marked logged and signed"
    );
    let revokes = all_entries(records.as_ref(), &scope)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::GrantRevoke)
        .count();
    assert_eq!(revokes, 1, "bug: a retried drain appended a second entry");

    let issue = athlete.issue(&grant).await;
    let jws = issue.jws.clone().expect("the receipt is signed");
    let (input, signature) = jws.rsplit_once('.').unwrap();
    let (header, body) = input.split_once('.').unwrap();
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
    assert_eq!(
        header,
        serde_json::json!({
            "alg": "EdDSA",
            "typ": "health-grant-receipt+jws",
            "kid": checkpoint_key_id(ISSUER).unwrap(),
            "iss": ISSUER,
        }),
        "bug: the protected header"
    );
    assert_eq!(
        URL_SAFE_NO_PAD.decode(body).unwrap(),
        issue.payload.as_bytes(),
        "bug: the JWS does not sign the stored bytes"
    );
    UnparsedPublicKey::new(&ED25519, signer.public_key())
        .verify(
            input.as_bytes(),
            &URL_SAFE_NO_PAD.decode(signature).unwrap(),
        )
        .expect("the receipt verifies under the checkpoint key");
}

/// A receipt the outbox can never log (its user id names no barrage scope)
/// sorts behind every receipt not yet tried once it has failed, so a page of
/// failures never holds the queue's head; after its last allowed attempt it
/// is poisoned and neither scan reads it again, while the receipts behind it
/// are logged and signed.
#[tokio::test]
async fn a_poisoned_receipt_at_the_head_never_stops_the_receipts_behind_it() {
    let athlete = athlete().await;
    let records = Arc::new(memory_records());
    let events = Arc::clone(&athlete.resources.common.repos.oauth_grant_events);
    let mut broken = athlete.grant(&client_id(), SCOPE, None);
    broken.user_id = "not-a-uuid".to_owned();
    broken.granted_at = Timestamp::now() - Elapsed::hours(1);
    let broken_id = athlete.approve(&broken).await;
    let signer = Arc::new(LocalSigner::random().unwrap());
    let worker = athlete.worker(records.clone(), Some(signer));

    let first = worker.run().await.unwrap();
    assert_eq!(
        (first.grant_receipts_logged, first.grant_receipts_signed),
        (0, 1),
        "bug: the broken receipt was logged, or not signed"
    );

    // A receipt written after the failure is read before it, even on a page
    // of one, though the failed receipt is older.
    let healthy = athlete
        .approve(&athlete.grant(&format!("{}-2", client_id()), SCOPE, None))
        .await;
    let head = events.list_unlogged(1).await.unwrap();
    assert_eq!(
        head.iter()
            .map(|r| r.client_grant_id.as_str())
            .collect::<Vec<_>>(),
        [healthy.as_str()],
        "bug: a failed receipt holds the head of the outbox"
    );

    for _ in 1..MAX_GRANT_RECEIPT_ATTEMPTS {
        worker.run().await.unwrap();
    }
    let unlogged = events.list_unlogged(500).await.unwrap();
    assert!(
        unlogged.iter().all(|r| r.client_grant_id != broken_id),
        "bug: a receipt failed {MAX_GRANT_RECEIPT_ATTEMPTS} times is still scanned"
    );
    let healthy_issue = athlete.issue(&healthy).await;
    assert!(
        healthy_issue.logged_at.is_some() && healthy_issue.jws.is_some(),
        "bug: the receipt behind the failing one was not logged and signed"
    );

    // Once poisoned, the broken receipt is out of both scans, and a receipt
    // written after it is logged and signed on the next tick.
    let unsigned = events.list_unsigned(500).await.unwrap();
    assert!(
        unsigned.iter().all(|r| r.client_grant_id != broken_id),
        "bug: a poisoned receipt is still in the signing scan"
    );
    let after = athlete
        .approve(&athlete.grant(&format!("{}-3", client_id()), SCOPE, None))
        .await;
    let tick = worker.run().await.unwrap();
    assert_eq!(
        (tick.grant_receipts_logged, tick.grant_receipts_signed),
        (1, 1),
        "bug: a poisoned receipt held back the receipt written after it"
    );
    let after_issue = athlete.issue(&after).await;
    assert!(
        after_issue.logged_at.is_some() && after_issue.jws.is_some(),
        "bug: the receipt written after the poisoned one was not logged and signed"
    );
}

/// An athlete with a live grant (`kept`) and one deleted with its client
/// (`deleted`), and a session of theirs.
async fn athlete_with_receipts() -> (Athlete, String, String, String) {
    let athlete = athlete().await;
    let client = client_id();
    let kept = athlete
        .approve(&athlete.grant(&client_id(), SCOPE, None))
        .await;
    let deleted = athlete.approve(&athlete.grant(&client, SCOPE, None)).await;
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .delete_client(&client)
        .await
        .unwrap();
    let session = common::generate_test_token(&athlete.resources, &athlete.user).await;
    (athlete, kept, deleted, session)
}

/// A session of another athlete on the same server.
async fn stranger_session(athlete: &Athlete) -> String {
    let email = format!("receipts_other_{}@example.com", Uuid::new_v4());
    let (_, other) = common::create_test_user_with_email(&athlete.resources.agent.database, &email)
        .await
        .unwrap();
    common::generate_test_token(&athlete.resources, &other).await
}

/// `GET uri` with `session`'s bearer, answering `status`, as JSON.
async fn get_json(athlete: &Athlete, uri: &str, session: &str, status: StatusCode) -> Value {
    AxumTestRequest::get(uri)
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&athlete.resources)))
        .await
        .assert_status(status)
        .json()
}

#[tokio::test]
async fn a_grants_receipts_route_serves_them_and_refuses_anothers() {
    let (athlete, kept, deleted, session) = athlete_with_receipts().await;

    // A grant deleted with its client keeps its receipts.
    let uri = format!("/api/me/oauth-grants/{deleted}/receipts");
    let listed = get_json(&athlete, &uri, &session, StatusCode::OK).await;
    let receipts = listed["receipts"].as_array().unwrap();
    let events: Vec<_> = receipts.iter().map(|r| r["event"].clone()).collect();
    assert_eq!(
        events,
        vec![Value::from("revoke"), Value::from("issue")],
        "bug: not the grant's receipts newest first: {listed}"
    );
    let issue = &receipts[1];
    assert_eq!(
        issue["payload"]["grant"]["id"],
        deleted.as_str(),
        "bug: the payload is not served"
    );
    assert!(issue["jws"].is_null(), "bug: an unsigned receipt has a JWS");
    assert_eq!(
        sha256_hex(&serde_json::to_string(&issue["payload"]).unwrap()),
        issue["payload_sha256"].as_str().unwrap(),
        "bug: the served payload is not the bytes its digest names"
    );

    // Another athlete's grant, an unknown one, or no session is refused.
    let other = stranger_session(&athlete).await;
    let kept_uri = format!("/api/me/oauth-grants/{kept}/receipts");
    get_json(&athlete, &kept_uri, &other, StatusCode::NOT_FOUND).await;
    let unknown = format!("/api/me/oauth-grants/{}/receipts", Uuid::new_v4());
    AxumTestRequest::get(&unknown)
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&athlete.resources)))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    AxumTestRequest::get(&kept_uri)
        .send(OAuthGrantsRoutes::routes(Arc::clone(&athlete.resources)))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_athletes_receipts_route_pages_every_receipt_of_theirs_only() {
    let (athlete, kept, deleted, session) = athlete_with_receipts().await;

    let all = get_json(
        &athlete,
        "/api/me/oauth-grant-receipts",
        &session,
        StatusCode::OK,
    )
    .await;
    let all = all["receipts"].as_array().unwrap();
    assert_eq!(all.len(), 3, "bug: the athlete-wide listing: {all:?}");
    let grants: Vec<_> = all.iter().map(|r| r["client_grant_id"].clone()).collect();
    assert!(
        grants.contains(&Value::from(deleted)) && grants.contains(&Value::from(kept)),
        "bug: a grant's receipts are missing: {grants:?}"
    );
    let page = get_json(
        &athlete,
        "/api/me/oauth-grant-receipts?limit=1&offset=1",
        &session,
        StatusCode::OK,
    )
    .await;
    let page = page["receipts"].as_array().unwrap();
    assert_eq!(page.len(), 1, "bug: the page size is not held");
    assert_eq!(
        page[0]["id"], all[1]["id"],
        "bug: the offset is not the listing's"
    );

    let other = stranger_session(&athlete).await;
    let others = get_json(
        &athlete,
        "/api/me/oauth-grant-receipts",
        &other,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        others["receipts"].as_array().map(Vec::len),
        Some(0),
        "bug: another athlete reads these receipts"
    );
}

/// Register `client_id` as a client, so the orphan delete keeps its grants.
async fn register(athlete: &Athlete, client_id: &str) {
    use pierre_core::models::OAuth2Client;
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .store_client(&OAuth2Client {
            id: Uuid::new_v4().to_string(),
            client_id: client_id.to_owned(),
            client_secret_hash: String::new(),
            redirect_uris: vec!["https://client.example/callback".to_owned()],
            grant_types: vec!["authorization_code".to_owned()],
            response_types: vec!["code".to_owned()],
            client_name: Some("Receipts client".to_owned()),
            client_uri: None,
            scope: None,
            created_at: Timestamp::now(),
            expires_at: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn the_baseline_gives_an_expired_grant_the_sweep_already_ended_its_issue() {
    let athlete = athlete().await;
    let past = Timestamp::now() - Elapsed::minutes(5);
    let client = client_id();
    // A registered client, so the orphan delete keeps the grant row.
    register(&athlete, &client).await;
    let grant = athlete
        .approve(&athlete.grant(&client, SCOPE, Some(past)))
        .await;
    // A grant given before receipts, which the hourly sweep reaches first.
    athlete.forget_receipts(&grant).await;
    assert_eq!(athlete.sweep().await, 1, "fixture: the sweep expires it");
    let events = &athlete.resources.common.repos.oauth_grant_events;

    assert_eq!(
        events.baseline_live_grants(500).await.unwrap(),
        1,
        "bug: an expired grant holding only its expire got no issue"
    );
    assert_eq!(
        events.baseline_live_grants(500).await.unwrap(),
        0,
        "bug: baselined twice"
    );
    let issue = athlete.issue(&grant).await;
    assert_eq!(
        (issue.actor, payload(&issue)["approval"]["baseline"].clone()),
        (GrantActor::Auto, Value::Bool(true)),
        "bug: not a baseline by the AS"
    );
    assert_eq!(
        athlete.ending(&grant).await.event,
        GrantEvent::Expire,
        "bug: the expire it held was lost"
    );
}

#[tokio::test]
async fn the_export_carries_each_receipt_its_consent_record_and_its_log_entry() {
    use pierre_routes_auth::AuthRoutes;

    let athlete = athlete().await;
    let details = serde_json::json!([
        {"type": "https://pdpp.dev/data-access", "selection_preset": "training_coach",
         "retention": {"max_duration": "P30D", "on_expiry": "delete"}},
        {"type": "https://dravr.ai/pdpp/health-profile/v0/health-constraints",
         "purpose_dpv": "https://w3id.org/dpv#ServiceProvision"}
    ]);
    let client = client_id();
    let mut asked = athlete.grant(&client, SCOPE, None);
    asked.authorization_details = Some(details.to_string());
    let grant = athlete.approve(&asked).await;
    athlete
        .resources
        .common
        .repos
        .oauth2_server
        .revoke_client_grant(
            &grant,
            &athlete.user_id,
            &athlete.tenant_id,
            GrantActor::User,
        )
        .await
        .unwrap();
    // The worker logs both receipts into the store the export reads.
    let records = Arc::clone(&athlete.resources.common.activity_records);
    let tick = athlete.worker(records, None).run().await.unwrap();
    assert_eq!(tick.grant_receipts_logged, 2, "fixture: both logged");

    let session = common::generate_test_token(&athlete.resources, &athlete.user).await;
    let response = AxumTestRequest::get("/api/user/export")
        .header("Authorization", &format!("Bearer {session}"))
        .send(AuthRoutes::routes(athlete.resources.auth_routes_context()))
        .await
        .assert_status(StatusCode::OK);
    let doc: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(doc["complete"], true, "bug: {doc}");

    let rows = doc["tables"]["oauth_grant_events"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        2,
        "bug: the receipts are not exported: {rows:?}"
    );
    let records = doc["tables"]["consent_records"].as_array().unwrap();
    assert_eq!(records.len(), 2, "bug: one record per receipt: {records:?}");
    let entries: Vec<String> = doc["tables"]["barrage.record_log"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|log| log["entries"].as_array().cloned().unwrap_or_default())
        .filter_map(|entry| entry["content"].as_str().map(str::to_owned))
        .collect();
    for receipt in athlete.receipts(&grant).await {
        let record = records
            .iter()
            .find(|record| record["@id"] == format!("urn:health-receipt:{}", receipt.id))
            .unwrap_or_else(|| panic!("bug: no record for receipt {}", receipt.id));
        let status = match receipt.event {
            GrantEvent::Issue | GrantEvent::StepUp => "dpv:ConsentGiven",
            GrantEvent::Revoke => "dpv:ConsentWithdrawn",
            GrantEvent::Expire | GrantEvent::Consume => "dpv:ConsentExpired",
        };
        assert_eq!(
            record["dpv:hasConsentStatus"]["@id"], status,
            "bug: {record}"
        );
        assert_eq!(
            (
                &record["dpv:hasDataSubject"]["@id"],
                &record["dpv:hasRecipient"]["@id"],
                &record["dpv:hasPurpose"]["@id"],
                &record["dpv:hasPersonalData"][0]["@id"],
            ),
            (
                &Value::from(format!("urn:dravr:user:{}", athlete.user_id)),
                &Value::from(client.clone()),
                &Value::from("https://w3id.org/dpv#ServiceProvision"),
                &Value::from("https://dravr.ai/pdpp/health-profile/v0/preset#training_coach"),
            ),
            "bug: the record does not project its receipt: {record}"
        );
        assert!(
            !record.to_string().contains("health.vault"),
            "bug: a record names a stream: {record}"
        );
        assert!(
            entries.contains(&receipt.payload_sha256),
            "bug: the exported log does not name receipt {}'s SHA-256",
            receipt.id
        );
    }
}
