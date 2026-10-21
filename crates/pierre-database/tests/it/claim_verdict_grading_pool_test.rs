// ABOUTME: Covers the blind-grading pool against whichever backend DATABASE_URL names (carnet#854)
// ABOUTME: Pins tenant scope, the window, the consent and provenance rules, their counts, and the newest-first cap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The pool is what a blind grader may be shown, so each rule that keeps a
//! verdict out is pinned by a verdict it keeps out, counted under its own
//! reason. `create_test_db` opens whichever `DATABASE_URL` names, so the
//! same assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{AgentCategory, CreateAgentRequest, Tenant, TenantId, User};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, GradingWithheld, VerdictLayer,
};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A tenant and its owner.
async fn seed_tenant(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
    let owner = User::new(
        format!("owner-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Owner".to_owned()),
    );
    let owner_id = repos.users.create(&owner).await.unwrap();
    let tenant = Tenant::new(
        "Grading Tenant".to_owned(),
        format!("grading-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        owner_id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    (owner_id, tenant.id)
}

/// An athlete with the given analytics consent and locale.
async fn athlete(repos: &RepositoryRegistry, analytics: bool, locale: &str) -> Uuid {
    let user = User::new(
        format!("athlete-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        None,
    );
    let id = repos.users.create(&user).await.unwrap();
    repos
        .users
        .update_analytics_consent(id, analytics)
        .await
        .unwrap();
    repos.users.update_locale(id, locale).await.unwrap();
    id
}

/// A group in `tenant_id` with `member` in it, sharing with the coach or not.
async fn join_group_sharing(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    owner_id: Uuid,
    member: Uuid,
    coach_sharing: bool,
) {
    let agent = repos
        .agents
        .create(
            owner_id,
            tenant_id,
            &CreateAgentRequest {
                title: "Group Coach".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            tenant_id,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                name: "Marathon crew".to_owned(),
                description: None,
                agent_id: agent.id.to_string(),
                owner_id,
                coach_user_id: Some(owner_id),
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 12,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: member,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: coach_sharing,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

async fn verdict(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user: Uuid,
    claim: &str,
    policy: TransportPolicy,
) -> ClaimVerdict {
    repos
        .claim_verdicts
        .insert_claim_verdict(&InsertClaimVerdictParams {
            tenant_id,
            user_id: &user.to_string(),
            agent_id: None,
            conversation_id: None,
            message_id: None,
            claim_text: claim,
            category: ClaimCategory::AthleteData,
            status: ClaimStatus::Unverifiable,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.0,
            layer_fired: VerdictLayer::AthleteData,
            explanation: None,
            evidence_refs: None,
            transport_policy: policy,
        })
        .await
        .unwrap()
}

/// Move a verdict's `created_at`, which the repository always stamps now.
async fn backdate(db: &Database, verdict_id: &str, at: DateTime<Utc>) {
    const SQL: &str = "UPDATE claim_verdicts SET created_at = $1 WHERE id = $2";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(at)
                .bind(verdict_id)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(SQL)
                .bind(at)
                .bind(verdict_id)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn the_pool_holds_only_what_a_grader_may_see_and_counts_the_rest() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (owner_id, tenant_id) = seed_tenant(repos).await;
    let (_, other_tenant) = seed_tenant(repos).await;

    let consenting = athlete(repos, true, "en").await;
    let silent = athlete(repos, false, "fr").await;
    let refused = athlete(repos, true, "fr").await;
    join_group_sharing(repos, tenant_id, owner_id, refused, false).await;
    let member = athlete(repos, true, "es").await;
    join_group_sharing(repos, tenant_id, owner_id, member, true).await;

    let any = TransportPolicy::AnyTransport;
    let a = verdict(repos, tenant_id, consenting, "You ran 21 km.", any).await;
    let b = verdict(repos, tenant_id, member, "You rode 60 km.", any).await;
    verdict(
        repos,
        tenant_id,
        consenting,
        "From a first-party-only reply.",
        TransportPolicy::FirstPartyOnly,
    )
    .await;
    verdict(repos, tenant_id, silent, "No analytics consent.", any).await;
    verdict(repos, tenant_id, refused, "Coach sharing withheld.", any).await;
    // Another tenant's verdict, and one from before the window.
    verdict(repos, other_tenant, consenting, "Other tenant.", any).await;
    let old = verdict(repos, tenant_id, consenting, "Too old.", any).await;
    backdate(&db, &old.id, Utc::now() - Duration::days(40)).await;

    let pool = repos
        .claim_verdicts
        .grading_pool(tenant_id, 30, 100)
        .await
        .unwrap();

    let mut ids: Vec<&str> = pool
        .candidates
        .iter()
        .map(|c| c.verdict.id.as_str())
        .collect();
    ids.sort_unstable();
    let mut expected = vec![a.id.as_str(), b.id.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert_eq!(pool.eligible, 2);
    assert_eq!(
        pool.withheld,
        GradingWithheld {
            first_party_only: 1,
            no_analytics_consent: 1,
            coach_sharing_withheld: 1,
        }
    );
    assert_eq!(pool.in_window, 5);
    assert_eq!(pool.window_days, 30);

    let locale_of = |id: &str| {
        pool.candidates
            .iter()
            .find(|c| c.verdict.id == id)
            .map(|c| c.locale.clone())
            .unwrap()
    };
    assert_eq!(locale_of(&a.id), "en");
    assert_eq!(locale_of(&b.id), "es");
    assert!(pool
        .candidates
        .iter()
        .all(|c| c.verdict.tenant_id == tenant_id.to_string()));
}

#[tokio::test]
async fn a_capped_read_keeps_the_newest_and_still_counts_every_eligible_verdict() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (_, tenant_id) = seed_tenant(repos).await;
    let consenting = athlete(repos, true, "fr").await;

    let any = TransportPolicy::AnyTransport;
    let oldest = verdict(repos, tenant_id, consenting, "Oldest.", any).await;
    let middle = verdict(repos, tenant_id, consenting, "Middle.", any).await;
    let newest = verdict(repos, tenant_id, consenting, "Newest.", any).await;
    backdate(&db, &oldest.id, Utc::now() - Duration::days(3)).await;
    backdate(&db, &middle.id, Utc::now() - Duration::days(2)).await;

    let pool = repos
        .claim_verdicts
        .grading_pool(tenant_id, 30, 2)
        .await
        .unwrap();
    let ids: Vec<&str> = pool
        .candidates
        .iter()
        .map(|c| c.verdict.id.as_str())
        .collect();
    assert_eq!(ids, [newest.id.as_str(), middle.id.as_str()]);
    assert_eq!(pool.eligible, 3);
    assert_eq!(pool.in_window, 3);
}

#[tokio::test]
async fn a_tenant_with_nothing_in_the_window_has_an_empty_pool() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (_, tenant_id) = seed_tenant(repos).await;
    let pool = repos
        .claim_verdicts
        .grading_pool(tenant_id, 30, 100)
        .await
        .unwrap();
    assert!(pool.candidates.is_empty());
    assert_eq!((pool.in_window, pool.eligible), (0, 0));
    assert_eq!(pool.withheld, GradingWithheld::default());
}
