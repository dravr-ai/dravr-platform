// ABOUTME: Covers the coaching harness memory repository against whichever backend DATABASE_URL names
// ABOUTME: Compaction blocks, user facts, agent notes, followups and sessions, each read back with real values
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `HarnessMemoryRepository` is emitted from one shared body for both
//! backends, so these assertions are what pins the two drivers to the same
//! behaviour: timestamps that round-trip, optional filters that narrow the
//! way they say, counters that count, and the state machines of notes,
//! followups and sessions.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{Pillar, Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::{
    InsertAgentFollowupParams, InsertAgentNoteParams, InsertCompactionBlockParams,
    MergeUserFactParams, UpsertUserFactParams,
};
use pierre_memory::{
    FactKind, FactSource, FollowupStatus, MemoryScope, PredicateCode, SessionStatus,
};
use uuid::Uuid;

/// The user, tenant and agent rows the memory tables' foreign keys resolve
/// against, created through the repositories so both backends accept them.
struct Seed {
    tenant: TenantId,
    user_id: String,
    agent_id: String,
}

async fn seed(db: &Database) -> Seed {
    let repos = db.repositories();
    let user = User::new(
        format!("memory-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Memory Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "Memory Tenant".to_owned(),
        format!("memory-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    let agent = repos
        .agents
        .create(
            user.id,
            tenant.id,
            &CreateAgentRequest {
                title: "Memory Coach".to_owned(),
                description: None,
                system_prompt: "You remember".to_owned(),
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
    Seed {
        tenant: tenant.id,
        user_id: user.id.to_string(),
        agent_id: agent.id.to_string(),
    }
}

/// Postgres keeps microseconds and `SQLite` keeps what sqlx wrote, so a
/// timestamp read back is compared to the one the repository returned at
/// well under a second, not to the nanosecond.
fn assert_same_instant(read: DateTime<Utc>, written: DateTime<Utc>, what: &str) {
    assert!(
        (read - written).num_milliseconds().abs() < 1000,
        "{what}: read {read} against written {written}"
    );
}

fn fact<'a>(
    seed: &'a Seed,
    kind: FactKind,
    predicate_code: PredicateCode,
    object: &'a str,
    source: FactSource,
    agent_id: Option<&'a str>,
) -> UpsertUserFactParams<'a> {
    UpsertUserFactParams {
        tenant_id: seed.tenant,
        user_id: &seed.user_id,
        agent_id,
        scope: MemoryScope::User,
        kind,
        pillar: Some(Pillar::TrainingAndMovement),
        predicate_code,
        object,
        confidence: 0.6,
        source,
        valid_until: None,
        source_msg_id: None,
    }
}

#[tokio::test]
async fn a_compaction_block_reads_back_in_conversation_order() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let seed = seed(&db).await;
    let conversation = repos
        .chat
        .create_conversation(
            &seed.user_id,
            seed.tenant,
            "compaction",
            "stub-model",
            None,
            None,
        )
        .await
        .unwrap();

    let first = repos
        .memory
        .insert_compaction_block(&InsertCompactionBlockParams {
            tenant_id: seed.tenant,
            conversation_id: &conversation.id,
            summary: "the athlete asked about tempo runs",
            summary_tokens: 12,
            original_tokens: 340,
            first_message_id: "m1",
            last_message_id: "m9",
        })
        .await
        .unwrap();
    let second = repos
        .memory
        .insert_compaction_block(&InsertCompactionBlockParams {
            tenant_id: seed.tenant,
            conversation_id: &conversation.id,
            summary: "then about recovery",
            summary_tokens: 7,
            original_tokens: 120,
            first_message_id: "m10",
            last_message_id: "m15",
        })
        .await
        .unwrap();

    let blocks = repos
        .memory
        .list_compaction_blocks(&conversation.id, seed.tenant)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].id, first.id);
    assert_eq!(blocks[0].summary, "the athlete asked about tempo runs");
    assert_eq!(blocks[0].summary_tokens, 12);
    assert_eq!(blocks[0].original_tokens, 340);
    assert_eq!(blocks[0].first_message_id, "m1");
    assert_eq!(blocks[0].last_message_id, "m9");
    assert_eq!(blocks[0].tenant_id, seed.tenant.to_string());
    assert_same_instant(blocks[0].created_at, first.created_at, "block created_at");
    assert_eq!(blocks[1].id, second.id);
    assert_eq!(blocks[1].summary, "then about recovery");

    let other_tenant = TenantId::generate();
    assert!(repos
        .memory
        .list_compaction_blocks(&conversation.id, other_tenant)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn user_facts_read_back_through_every_filter_and_delete() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let goal = memory
        .upsert_user_fact(&fact(
            &seed,
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "a sub-4 marathon",
            FactSource::Conversation,
            Some(&seed.agent_id),
        ))
        .await
        .unwrap();
    let preference = memory
        .upsert_user_fact(&fact(
            &seed,
            FactKind::Preference,
            PredicateCode::Prefer,
            "morning runs",
            FactSource::Conversation,
            None,
        ))
        .await
        .unwrap();

    let stored = memory
        .get_user_fact(&goal.id, seed.tenant, &seed.user_id)
        .await
        .unwrap()
        .expect("the goal fact reads back");
    assert_eq!(stored.id, goal.id);
    assert_eq!(stored.tenant_id, seed.tenant.to_string());
    assert_eq!(stored.user_id, seed.user_id);
    assert_eq!(stored.agent_id.as_deref(), Some(seed.agent_id.as_str()));
    assert_eq!(stored.scope, MemoryScope::User);
    assert_eq!(stored.kind, FactKind::Goal);
    assert_eq!(stored.pillar, Some(Pillar::TrainingAndMovement));
    assert_eq!(stored.predicate_code, PredicateCode::TrainingFor);
    assert_eq!(stored.object, "a sub-4 marathon");
    assert!((stored.confidence - 0.6).abs() < 1e-6);
    assert_eq!(stored.source, FactSource::Conversation);
    assert_eq!(stored.valid_until, None);
    assert_eq!(stored.source_msg_id, None);
    assert_same_instant(stored.created_at, goal.created_at, "fact created_at");
    assert_same_instant(stored.updated_at, goal.updated_at, "fact updated_at");

    let stranger = Uuid::new_v4().to_string();
    assert!(memory
        .get_user_fact(&goal.id, seed.tenant, &stranger)
        .await
        .unwrap()
        .is_none());

    let ids = |facts: &[pierre_memory::UserFact]| -> Vec<String> {
        let mut ids: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();
        ids.sort();
        ids
    };
    let mut both = vec![goal.id.clone(), preference.id.clone()];
    both.sort();

    let all = memory
        .list_user_facts(seed.tenant, &seed.user_id, None, None, 10)
        .await
        .unwrap();
    assert_eq!(ids(&all), both);

    let by_agent = memory
        .list_user_facts(seed.tenant, &seed.user_id, Some(&seed.agent_id), None, 10)
        .await
        .unwrap();
    assert_eq!(ids(&by_agent), vec![goal.id.clone()]);

    let by_kind = memory
        .list_user_facts(
            seed.tenant,
            &seed.user_id,
            None,
            Some(FactKind::Preference),
            10,
        )
        .await
        .unwrap();
    assert_eq!(ids(&by_kind), vec![preference.id.clone()]);

    let by_agent_and_kind = memory
        .list_user_facts(
            seed.tenant,
            &seed.user_id,
            Some(&seed.agent_id),
            Some(FactKind::Goal),
            10,
        )
        .await
        .unwrap();
    assert_eq!(ids(&by_agent_and_kind), vec![goal.id.clone()]);
    assert!(memory
        .list_user_facts(
            seed.tenant,
            &seed.user_id,
            Some(&seed.agent_id),
            Some(FactKind::Preference),
            10,
        )
        .await
        .unwrap()
        .is_empty());

    let limited = memory
        .list_user_facts(seed.tenant, &seed.user_id, None, None, 1)
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);

    assert!(memory
        .delete_user_fact(&preference.id, seed.tenant, &seed.user_id)
        .await
        .unwrap());
    assert!(!memory
        .delete_user_fact(&preference.id, seed.tenant, &seed.user_id)
        .await
        .unwrap());
    let remaining = memory
        .list_user_facts(seed.tenant, &seed.user_id, None, None, 10)
        .await
        .unwrap();
    assert_eq!(ids(&remaining), vec![goal.id]);
}

#[tokio::test]
async fn listing_by_source_drops_a_fact_past_its_horizon() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let live = memory
        .upsert_user_fact(&UpsertUserFactParams {
            valid_until: Some(Utc::now() + Duration::days(30)),
            ..fact(
                &seed,
                FactKind::Goal,
                PredicateCode::TargetRace,
                "the autumn half",
                FactSource::Onboarding,
                None,
            )
        })
        .await
        .unwrap();
    memory
        .upsert_user_fact(&UpsertUserFactParams {
            valid_until: Some(Utc::now() - Duration::days(1)),
            ..fact(
                &seed,
                FactKind::Goal,
                PredicateCode::AimThisSeason,
                "a spring 10k, already run",
                FactSource::Onboarding,
                None,
            )
        })
        .await
        .unwrap();
    let no_horizon = memory
        .upsert_user_fact(&fact(
            &seed,
            FactKind::Schedule,
            PredicateCode::CanTrainOn,
            "weekday evenings",
            FactSource::Onboarding,
            None,
        ))
        .await
        .unwrap();
    memory
        .upsert_user_fact(&fact(
            &seed,
            FactKind::Preference,
            PredicateCode::Prefer,
            "trail over road",
            FactSource::Conversation,
            None,
        ))
        .await
        .unwrap();

    let onboarding = memory
        .list_user_facts_by_source(seed.tenant, &seed.user_id, FactSource::Onboarding, 10)
        .await
        .unwrap();
    let mut got: Vec<&str> = onboarding.iter().map(|f| f.id.as_str()).collect();
    got.sort_unstable();
    let mut want = vec![live.id.as_str(), no_horizon.id.as_str()];
    want.sort_unstable();
    assert_eq!(got, want);
    let horizon = onboarding
        .iter()
        .find(|f| f.id == live.id)
        .and_then(|f| f.valid_until)
        .expect("the live fact keeps its horizon");
    assert_same_instant(horizon, live.valid_until.unwrap(), "valid_until");
}

#[tokio::test]
async fn a_merge_only_raises_confidence_and_keeps_the_anchors_words() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let anchor = memory
        .upsert_user_fact(&UpsertUserFactParams {
            source_msg_id: Some("msg-1"),
            ..fact(
                &seed,
                FactKind::Goal,
                PredicateCode::TrainingFor,
                "Boston next April",
                FactSource::Conversation,
                None,
            )
        })
        .await
        .unwrap();

    let raised = memory
        .merge_user_fact(&MergeUserFactParams {
            tenant_id: seed.tenant,
            fact_id: &anchor.id,
            source_msg_id: Some("msg-2"),
            confidence: 0.9,
        })
        .await
        .unwrap()
        .expect("the anchor exists");
    assert_eq!(raised.object, "Boston next April");
    assert!((raised.confidence - 0.9).abs() < 1e-6);
    assert_eq!(raised.source_msg_id.as_deref(), Some("msg-2"));
    assert!(raised.updated_at >= anchor.updated_at - Duration::seconds(1));

    let held = memory
        .merge_user_fact(&MergeUserFactParams {
            tenant_id: seed.tenant,
            fact_id: &anchor.id,
            source_msg_id: None,
            confidence: 0.3,
        })
        .await
        .unwrap()
        .unwrap();
    assert!(
        (held.confidence - 0.9).abs() < 1e-6,
        "a poorer restatement never lowers confidence, got {}",
        held.confidence
    );
    assert_eq!(
        held.source_msg_id.as_deref(),
        Some("msg-2"),
        "a restatement without a message keeps the last one"
    );

    assert!(memory
        .merge_user_fact(&MergeUserFactParams {
            tenant_id: TenantId::generate(),
            fact_id: &anchor.id,
            source_msg_id: None,
            confidence: 1.0,
        })
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn expiring_onboarding_facts_honours_every_narrowing() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let training = memory
        .upsert_user_fact(&fact(
            &seed,
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "a marathon",
            FactSource::Onboarding,
            None,
        ))
        .await
        .unwrap();
    let fuelling = memory
        .upsert_user_fact(&UpsertUserFactParams {
            pillar: Some(Pillar::Fuelling),
            ..fact(
                &seed,
                FactKind::Preference,
                PredicateCode::Avoid,
                "gels",
                FactSource::Onboarding,
                None,
            )
        })
        .await
        .unwrap();
    let conversation = memory
        .upsert_user_fact(&fact(
            &seed,
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "said in chat",
            FactSource::Conversation,
            None,
        ))
        .await
        .unwrap();

    let none_before_creation = memory
        .expire_onboarding_facts(
            seed.tenant,
            &seed.user_id,
            None,
            None,
            Some(training.created_at - Duration::hours(1)),
            None,
        )
        .await
        .unwrap();
    assert_eq!(none_before_creation, 0);

    let by_predicate = memory
        .expire_onboarding_facts(
            seed.tenant,
            &seed.user_id,
            None,
            None,
            None,
            Some(PredicateCode::Avoid),
        )
        .await
        .unwrap();
    assert_eq!(by_predicate, 1);
    let fuelling_now = memory
        .get_user_fact(&fuelling.id, seed.tenant, &seed.user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fuelling_now.valid_until.is_some_and(|v| v <= Utc::now()),
        "the expired fact carries a horizon in the past"
    );
    assert!(fuelling_now.updated_at > fuelling.updated_at - Duration::seconds(1));

    let by_pillar_wrong = memory
        .expire_onboarding_facts(
            seed.tenant,
            &seed.user_id,
            Some(Pillar::SleepAndRecovery),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(by_pillar_wrong, 0);

    let by_window = memory
        .expire_onboarding_facts(
            seed.tenant,
            &seed.user_id,
            Some(Pillar::TrainingAndMovement),
            Some(training.created_at - Duration::hours(1)),
            Some(Utc::now() + Duration::hours(1)),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        by_window, 1,
        "only the still-valid training fact remains to expire"
    );
    let training_now = memory
        .get_user_fact(&training.id, seed.tenant, &seed.user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(training_now.valid_until.is_some());

    let again = memory
        .expire_onboarding_facts(seed.tenant, &seed.user_id, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(again, 0, "an expired fact is not expired twice");

    let chat = memory
        .get_user_fact(&conversation.id, seed.tenant, &seed.user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        chat.valid_until, None,
        "a conversation fact is never touched"
    );
}

#[tokio::test]
async fn fact_metrics_count_the_tenant_by_kind_and_recency() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let empty = memory.count_user_facts_metrics(seed.tenant).await.unwrap();
    assert_eq!(empty.total_facts, 0);
    assert_eq!(empty.distinct_users, 0);
    assert_eq!(empty.facts_last_24h, 0);
    assert_eq!(empty.facts_last_7d, 0);
    assert!(empty.facts_by_kind.is_empty());
    assert_eq!(empty.newest_updated_at, None);

    for (kind, code, object) in [
        (FactKind::Goal, PredicateCode::TrainingFor, "a marathon"),
        (FactKind::Goal, PredicateCode::TargetRace, "the city half"),
        (
            FactKind::Injury,
            PredicateCode::RecoveringFrom,
            "a sore calf",
        ),
    ] {
        memory
            .upsert_user_fact(&fact(
                &seed,
                kind,
                code,
                object,
                FactSource::Conversation,
                None,
            ))
            .await
            .unwrap();
    }
    let other_user = Uuid::new_v4().to_string();
    let newest = memory
        .upsert_user_fact(&UpsertUserFactParams {
            user_id: &other_user,
            ..fact(
                &seed,
                FactKind::Equipment,
                PredicateCode::Own,
                "a power meter",
                FactSource::Device,
                None,
            )
        })
        .await
        .unwrap();

    let metrics = memory.count_user_facts_metrics(seed.tenant).await.unwrap();
    assert_eq!(metrics.total_facts, 4);
    assert_eq!(metrics.distinct_users, 2);
    assert_eq!(metrics.facts_last_24h, 4);
    assert_eq!(metrics.facts_last_7d, 4);
    assert_eq!(metrics.facts_by_kind.get("goal"), Some(&2));
    assert_eq!(metrics.facts_by_kind.get("injury"), Some(&1));
    assert_eq!(metrics.facts_by_kind.get("equipment"), Some(&1));
    assert_eq!(metrics.facts_by_kind.len(), 3);
    assert_same_instant(
        metrics.newest_updated_at.expect("four facts have a newest"),
        newest.updated_at,
        "newest_updated_at",
    );

    let elsewhere = memory
        .count_user_facts_metrics(TenantId::generate())
        .await
        .unwrap();
    assert_eq!(elsewhere.total_facts, 0);
}

#[tokio::test]
async fn a_suppressed_note_leaves_recall_and_stays_in_the_audit_list() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let note = memory
        .insert_agent_note(&InsertAgentNoteParams {
            tenant_id: seed.tenant,
            user_id: &seed.user_id,
            agent_id: &seed.agent_id,
            conversation_id: None,
            scope: MemoryScope::User,
            content: "prefers short answers",
        })
        .await
        .unwrap();
    assert!(!note.suppressed);

    let recalled = memory
        .list_agent_notes(seed.tenant, &seed.user_id, &seed.agent_id, 10)
        .await
        .unwrap();
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, note.id);
    assert_eq!(recalled[0].content, "prefers short answers");
    assert_eq!(recalled[0].scope, MemoryScope::User);
    assert_eq!(recalled[0].agent_id, seed.agent_id);
    assert_eq!(recalled[0].conversation_id, None);
    assert!(!recalled[0].suppressed);
    assert_same_instant(recalled[0].created_at, note.created_at, "note created_at");

    assert!(memory
        .set_agent_note_suppressed(&note.id, seed.tenant, true, "admin@example.com")
        .await
        .unwrap());
    assert!(
        !memory
            .set_agent_note_suppressed(&note.id, seed.tenant, true, "admin@example.com")
            .await
            .unwrap(),
        "suppressing twice changes nothing"
    );

    assert!(memory
        .list_agent_notes(seed.tenant, &seed.user_id, &seed.agent_id, 10)
        .await
        .unwrap()
        .is_empty());
    let audited = memory
        .list_agent_notes_for_tenant(seed.tenant, 10)
        .await
        .unwrap();
    assert_eq!(audited.len(), 1);
    assert!(audited[0].suppressed);
    assert!(audited[0].updated_at >= note.updated_at - Duration::seconds(1));

    assert!(memory
        .set_agent_note_suppressed(&note.id, seed.tenant, false, "admin@example.com")
        .await
        .unwrap());
    let back = memory
        .list_agent_notes(seed.tenant, &seed.user_id, &seed.agent_id, 10)
        .await
        .unwrap();
    assert_eq!(back.len(), 1);
    assert!(!back[0].suppressed);

    assert!(!memory
        .set_agent_note_suppressed(&note.id, TenantId::generate(), true, "admin@example.com")
        .await
        .unwrap());
}

#[tokio::test]
async fn followups_move_from_pending_to_delivered_or_cancelled_once() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;
    let now = Utc::now();

    let overdue = memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: seed.tenant,
            user_id: &seed.user_id,
            agent_id: &seed.agent_id,
            conversation_id: None,
            content: "ask how the long run went",
            due_at: Some(now - Duration::hours(2)),
        })
        .await
        .unwrap();
    let later = memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: seed.tenant,
            user_id: &seed.user_id,
            agent_id: &seed.agent_id,
            conversation_id: None,
            content: "check the race entry",
            due_at: Some(now + Duration::days(3)),
        })
        .await
        .unwrap();
    let undated = memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: seed.tenant,
            user_id: &seed.user_id,
            agent_id: &seed.agent_id,
            conversation_id: None,
            content: "mention the new shoes",
            due_at: None,
        })
        .await
        .unwrap();
    assert_eq!(overdue.status, FollowupStatus::Pending);
    assert_eq!(undated.due_at, None);

    let pending = memory
        .list_pending_followups(seed.tenant, &seed.user_id, &seed.agent_id)
        .await
        .unwrap();
    let order: Vec<&str> = pending.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        order,
        vec![overdue.id.as_str(), later.id.as_str(), undated.id.as_str()],
        "soonest due first, undated last"
    );
    assert_eq!(pending[0].content, "ask how the long run went");
    assert_same_instant(
        pending[0]
            .due_at
            .expect("the overdue followup has a due time"),
        now - Duration::hours(2),
        "due_at",
    );
    assert_eq!(pending[0].delivered_at, None);

    let for_tenant = memory
        .list_pending_followups_for_tenant(seed.tenant, 2)
        .await
        .unwrap();
    assert_eq!(for_tenant.len(), 2);
    assert_eq!(for_tenant[0].id, overdue.id);

    let due = memory.list_due_followups(now, 10).await.unwrap();
    let due_here: Vec<&str> = due
        .iter()
        .filter(|f| f.tenant_id == seed.tenant.to_string())
        .map(|f| f.id.as_str())
        .collect();
    assert_eq!(
        due_here,
        vec![overdue.id.as_str()],
        "only a dated, past-due followup is due"
    );

    assert!(memory
        .mark_followup_delivered(&overdue.id, seed.tenant)
        .await
        .unwrap());
    assert!(
        !memory
            .mark_followup_delivered(&overdue.id, seed.tenant)
            .await
            .unwrap(),
        "a delivered followup is not delivered twice"
    );
    assert!(memory
        .cancel_followup(&later.id, seed.tenant)
        .await
        .unwrap());
    assert!(!memory
        .cancel_followup(&later.id, seed.tenant)
        .await
        .unwrap());
    assert!(
        !memory
            .mark_followup_delivered(&later.id, seed.tenant)
            .await
            .unwrap(),
        "a cancelled followup cannot be delivered"
    );

    let left = memory
        .list_pending_followups(seed.tenant, &seed.user_id, &seed.agent_id)
        .await
        .unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, undated.id);
    assert!(memory
        .list_due_followups(now, 10)
        .await
        .unwrap()
        .iter()
        .all(|f| f.id != overdue.id));
}

#[tokio::test]
async fn a_session_is_reused_until_archived_then_reopened() {
    let db = create_test_db().await.unwrap();
    let memory = db.repositories().memory;
    let seed = seed(&db).await;

    let opened = memory
        .get_or_open_agent_session(seed.tenant, &seed.user_id, &seed.agent_id)
        .await
        .unwrap();
    assert_eq!(opened.status, SessionStatus::Active);
    assert_eq!(opened.last_turn_at, None);
    assert_eq!(opened.archived_at, None);
    assert_eq!(opened.agent_id, seed.agent_id);

    let same = memory
        .get_or_open_agent_session(seed.tenant, &seed.user_id, &seed.agent_id)
        .await
        .unwrap();
    assert_eq!(same.id, opened.id, "an active session is reused");
    assert_same_instant(same.opened_at, opened.opened_at, "opened_at");
    assert_same_instant(same.created_at, opened.created_at, "created_at");

    memory
        .touch_agent_session(&opened.id, seed.tenant)
        .await
        .unwrap();
    let touched = memory
        .get_or_open_agent_session(seed.tenant, &seed.user_id, &seed.agent_id)
        .await
        .unwrap();
    assert_eq!(touched.id, opened.id);
    let last_turn = touched.last_turn_at.expect("a touch records the turn");
    assert!(last_turn >= opened.opened_at - Duration::seconds(1));
    assert!(touched.updated_at >= last_turn - Duration::seconds(1));

    assert!(memory
        .archive_agent_session(&opened.id, seed.tenant)
        .await
        .unwrap());
    assert!(
        !memory
            .archive_agent_session(&opened.id, seed.tenant)
            .await
            .unwrap(),
        "an archived session is not archived twice"
    );

    let reopened = memory
        .get_or_open_agent_session(seed.tenant, &seed.user_id, &seed.agent_id)
        .await
        .unwrap();
    assert_ne!(reopened.id, opened.id, "an archived session is replaced");
    assert_eq!(reopened.status, SessionStatus::Active);
    assert_eq!(reopened.last_turn_at, None);

    assert!(!memory
        .archive_agent_session(&reopened.id, TenantId::generate())
        .await
        .unwrap());
}
