// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A fact the agent inferred is a draft no prompt reads until the athlete keeps it on the review list
// ABOUTME: Pins the idempotent write, the draft-only list, approval, and the refusal of a superseded draft

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::memory::MemoryRoutes;
use pierre_memory::{
    EvidenceSource, FactKind, FactProposal, FactSource, FactStatus, MemoryScope, PredicateCode,
    ProposedClaim, UserFact,
};
use serde_json::Value;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;

struct Athlete {
    resources: Arc<ServerContext>,
    token: String,
    user_id: String,
    tenant_id: TenantId,
}

async fn athlete() -> Athlete {
    let resources = create_test_server_resources().await.unwrap();
    let (user_uuid, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let tenant_id = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user_uuid)
        .await
        .unwrap()
        .first()
        .expect("the test user has a tenant")
        .id;
    Athlete {
        resources,
        token,
        user_id: user_uuid.to_string(),
        tenant_id,
    }
}

impl Athlete {
    async fn propose(
        &self,
        kind: FactKind,
        code: PredicateCode,
        object: &str,
        stated_by_athlete: bool,
        message: &str,
    ) -> UserFact {
        let claim = ProposedClaim {
            user_id: &self.user_id,
            kind,
            predicate_code: code,
            object,
        };
        let proposal = FactProposal::from_extraction(
            claim,
            stated_by_athlete,
            false,
            Some(object),
            message,
            "",
        );
        self.resources
            .agent
            .database
            .repositories()
            .memory
            .propose_user_fact(
                &UpsertUserFactParams {
                    tenant_id: self.tenant_id,
                    user_id: &self.user_id,
                    agent_id: None,
                    scope: MemoryScope::User,
                    kind,
                    pillar: None,
                    predicate_code: code,
                    object,
                    confidence: 0.8,
                    source: FactSource::Conversation,
                    valid_until: None,
                    source_msg_id: None,
                    transport_policy: TransportPolicy::AnyTransport,
                },
                &proposal,
            )
            .await
            .unwrap()
    }

    async fn get(&self, uri: &str) -> Value {
        let resp = AxumTestRequest::get(uri)
            .header("Authorization", &self.token)
            .send(MemoryRoutes::routes(Arc::clone(&self.resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        resp.json()
    }

    async fn approve(&self, fact_id: &str) -> StatusCode {
        AxumTestRequest::post(&format!("/api/memory/facts/{fact_id}/approve"))
            .header("Authorization", &self.token)
            .send(MemoryRoutes::routes(Arc::clone(&self.resources)))
            .await
            .status_code()
    }
}

fn objects(body: &Value) -> Vec<&str> {
    body["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["object"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_draft_reaches_only_the_review_list() {
    let a = athlete().await;
    let kept = a
        .propose(
            FactKind::Goal,
            PredicateCode::WorkingToward,
            "a sub-3 marathon",
            true,
            "I want a sub-3 marathon",
        )
        .await;
    let draft = a
        .propose(
            FactKind::Physiology,
            PredicateCode::States,
            "FTP around 250 W",
            true,
            "I held 250 for twenty minutes",
        )
        .await;
    assert_eq!(kept.status, FactStatus::Approved);
    assert_eq!(draft.status, FactStatus::Draft);
    let evidence = draft.evidence.clone().expect("a draft quotes its source");
    assert_eq!(evidence.quote, "I held 250 for twenty minutes");
    assert_eq!(evidence.source, EvidenceSource::Athlete);

    // Every read a prompt goes through keeps to approved facts.
    let memory = &a.resources.agent.database.repositories().memory;
    let recalled = memory
        .list_user_facts(
            a.tenant_id,
            &a.user_id,
            None,
            None,
            50,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert_eq!(
        recalled.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
        [kept.id.as_str()]
    );
    let by_source = memory
        .list_user_facts_by_source(
            a.tenant_id,
            &a.user_id,
            FactSource::Conversation,
            50,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert!(by_source.iter().all(|f| f.id != draft.id));

    assert_eq!(
        objects(&a.get("/api/memory/facts").await),
        ["a sub-3 marathon"]
    );
    let drafts = a.get("/api/memory/facts?status=draft").await;
    assert_eq!(objects(&drafts), ["FTP around 250 W"]);
    let row = &drafts["facts"][0];
    assert_eq!(row["status"], "draft");
    assert_eq!(row["evidence"], "I held 250 for twenty minutes");
    assert_eq!(row["evidence_source"], "athlete");
}

#[tokio::test]
async fn the_same_claim_written_twice_is_one_row_and_can_only_be_promoted() {
    let a = athlete().await;
    let first = a
        .propose(
            FactKind::Equipment,
            PredicateCode::Own,
            "a gravel bike",
            false,
            "",
        )
        .await;
    let again = a
        .propose(
            FactKind::Equipment,
            PredicateCode::Own,
            "A  Gravel bike",
            false,
            "",
        )
        .await;
    assert_eq!(
        first.id, again.id,
        "a replayed write lands on the first row"
    );
    assert_eq!(again.status, FactStatus::Draft);

    let stated = a
        .propose(
            FactKind::Equipment,
            PredicateCode::Own,
            "a gravel bike",
            true,
            "I ride a gravel bike most days",
        )
        .await;
    assert_eq!(stated.id, first.id);
    assert_eq!(
        stated.status,
        FactStatus::Approved,
        "the athlete's own words promote it"
    );
    assert_eq!(
        stated.evidence.unwrap().quote,
        "a gravel bike",
        "and their quote replaces the inferred one"
    );

    let unquoted = a
        .propose(
            FactKind::Equipment,
            PredicateCode::Own,
            "a gravel bike",
            false,
            "",
        )
        .await;
    assert_eq!(
        unquoted.status,
        FactStatus::Approved,
        "a later draft never demotes it"
    );
}

#[tokio::test]
async fn keeping_a_draft_puts_it_in_memory() {
    let a = athlete().await;
    let draft = a
        .propose(
            FactKind::Preference,
            PredicateCode::Prefer,
            "morning sessions",
            false,
            "",
        )
        .await;
    assert_eq!(a.approve(&draft.id).await, StatusCode::OK);
    assert_eq!(
        objects(&a.get("/api/memory/facts").await),
        ["morning sessions"]
    );
    let drafts = a.get("/api/memory/facts?status=draft").await;
    assert_eq!(objects(&drafts), Vec::<&str>::new());
    assert_eq!(
        a.approve(&draft.id).await,
        StatusCode::OK,
        "keeping twice is harmless"
    );
    assert_eq!(a.approve("no-such-fact").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_draft_overtaken_by_a_newer_fact_cannot_be_kept() {
    let a = athlete().await;
    let draft = a
        .propose(
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "a spring half marathon",
            false,
            "",
        )
        .await;
    a.propose(
        FactKind::Goal,
        PredicateCode::TrainingFor,
        "an autumn marathon",
        true,
        "I'm training for an autumn marathon now",
    )
    .await;

    assert_eq!(a.approve(&draft.id).await, StatusCode::CONFLICT);
    assert_eq!(
        objects(&a.get("/api/memory/facts?status=draft").await),
        ["a spring half marathon"],
        "the stale draft stays a draft for the athlete to discard"
    );
}
