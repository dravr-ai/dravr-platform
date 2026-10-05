// ABOUTME: Every derived-content store keeps the first-party-only stamp it is written with, on SQLite and Postgres
// ABOUTME: Merges and upserts only ever tighten a stamp, and a stamped playbook never feeds the cross-athlete priors

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Storage-layer checks for carnet#769: each store a reader withholds stamped
//! rows from has to hand the stamp back exactly as written, and a write that
//! folds into an existing row must never loosen it. Runs on the in-memory
//! `SQLite` fixture locally and against Postgres in the PG lane.

use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::time::sleep;

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{
    AddMessageParams, CalendarEventSource, MessageRecord, PrescribedWorkout, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    InsertAgentFollowupParams, InsertAgentNoteParams, InsertClaimVerdictParams,
    InsertCompactionBlockParams, MergeUserFactParams, PlanOutlineInput, PlanWeekInput,
    RecordedOutcome, SavePlanBundleParams, UpsertUserFactParams,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_memory::playbooks::{
    AdviceStatus, Band, Intervention, InterventionKind, LabelSource, MetricBaseline, OutcomeLabel,
    OutcomeMetric, PendingAdvice, TriggerKind, TriggerPattern,
};
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use uuid::Uuid;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

use TransportPolicy::{AnyTransport, FirstPartyOnly};

#[tokio::test]
async fn chat_rows_keep_their_stamp_through_every_read() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    let user = user_id.to_string();
    let conversation = repos
        .chat
        .create_conversation(&user, tenant, "Week", "model", None, None)
        .await
        .unwrap();
    for (content, policy) in [("open", AnyTransport), ("stamped", FirstPartyOnly)] {
        let written = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: tenant,
                conversation_id: &conversation.id,
                user_id: &user,
                role: "assistant",
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: policy,
            })
            .await
            .unwrap();
        assert_eq!(written.transport_policy, policy);
    }

    // Both rows can share a timestamp, so they are compared in content order.
    let stamps = |rows: Vec<MessageRecord>| {
        let mut stamps: Vec<_> = rows
            .into_iter()
            .map(|m| (m.content, m.transport_policy))
            .collect();
        stamps.sort_by(|a, b| a.0.cmp(&b.0));
        stamps
    };
    let expected = vec![
        ("open".to_owned(), AnyTransport),
        ("stamped".to_owned(), FirstPartyOnly),
    ];
    let all = repos
        .chat
        .get_messages(&conversation.id, &user, tenant)
        .await
        .unwrap();
    assert_eq!(stamps(all), expected);
    let recent = repos
        .chat
        .get_recent_messages(
            &conversation.id,
            &user,
            tenant,
            10,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert_eq!(stamps(recent), expected);

    let page = repos
        .chat
        .list_conversations(&user, tenant, 10, 0, TransportPolicy::FirstPartyOnly)
        .await
        .unwrap();
    let last = page.items[0].last_message.as_ref().expect("a preview");
    let previewed = expected
        .iter()
        .find(|(content, _)| *content == last.content_head)
        .expect("the preview is one of the rows");
    assert_eq!(
        last.transport_policy, previewed.1,
        "the previewed row's stamp"
    );

    let walked = repos
        .chat
        .list_assistant_messages_after(tenant, None, None, 10)
        .await
        .unwrap();
    let mut walked: Vec<(String, TransportPolicy)> = walked
        .into_iter()
        .map(|m| (m.content, m.transport_policy))
        .collect();
    walked.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(walked, expected, "the verdict backfill reads the stamp too");
}

#[tokio::test]
async fn memory_rows_keep_their_stamp_and_a_restatement_only_tightens_it() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    let user = user_id.to_string();
    let conversation = repos
        .chat
        .create_conversation(&user, tenant, "Long thread", "model", None, None)
        .await
        .unwrap();

    let block = repos
        .memory
        .insert_compaction_block(&InsertCompactionBlockParams {
            tenant_id: tenant,
            conversation_id: &conversation.id,
            summary: "summary of stamped turns",
            summary_tokens: 4,
            original_tokens: 40,
            first_message_id: "m1",
            last_message_id: "m9",
            transport_policy: FirstPartyOnly,
        })
        .await
        .unwrap();
    assert_eq!(block.transport_policy, FirstPartyOnly);
    let blocks = repos
        .memory
        .list_compaction_blocks(&conversation.id, tenant)
        .await
        .unwrap();
    assert_eq!(blocks[0].transport_policy, FirstPartyOnly);

    let fact = repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user,
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: None,
            predicate_code: PredicateCode::Prefer,
            object: "hill repeats",
            confidence: 0.7,
            source: FactSource::Conversation,
            valid_until: None,
            source_msg_id: None,
            transport_policy: AnyTransport,
        })
        .await
        .unwrap();
    assert_eq!(fact.transport_policy, AnyTransport);
    let merge = |policy| MergeUserFactParams {
        tenant_id: tenant,
        fact_id: &fact.id,
        source_msg_id: None,
        confidence: 0.8,
        transport_policy: policy,
    };
    let restated = repos
        .memory
        .merge_user_fact(&merge(FirstPartyOnly))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restated.transport_policy, FirstPartyOnly,
        "restated from first-party-only content, it now stands for it"
    );
    let restated_again = repos
        .memory
        .merge_user_fact(&merge(AnyTransport))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restated_again.transport_policy, FirstPartyOnly,
        "an open restatement never loosens the stamp"
    );
    let listed = repos
        .memory
        .list_user_facts(
            tenant,
            &user,
            None,
            None,
            10,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert_eq!(listed[0].transport_policy, FirstPartyOnly);

    let agent = repos
        .agents
        .create(
            user_id,
            tenant,
            &CreateAgentRequest {
                title: "Hill Coach".to_owned(),
                description: None,
                system_prompt: "You coach climbing.".to_owned(),
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
    let agent_id = agent.id.to_string();

    repos
        .memory
        .insert_agent_note(&InsertAgentNoteParams {
            tenant_id: tenant,
            user_id: &user,
            agent_id: &agent_id,
            conversation_id: None,
            scope: MemoryScope::User,
            content: "prefers the relay's hill loop",
            transport_policy: FirstPartyOnly,
        })
        .await
        .unwrap();
    let notes = repos
        .memory
        .list_agent_notes(
            tenant,
            &user,
            &agent_id,
            10,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert_eq!(notes[0].transport_policy, FirstPartyOnly);

    repos
        .memory
        .insert_agent_followup(&InsertAgentFollowupParams {
            tenant_id: tenant,
            user_id: &user,
            agent_id: &agent_id,
            conversation_id: None,
            content: "ask how the hill loop went",
            due_at: None,
            transport_policy: FirstPartyOnly,
        })
        .await
        .unwrap();
    let followups = repos
        .memory
        .list_pending_followups(tenant, &user, &agent_id)
        .await
        .unwrap();
    assert_eq!(followups[0].transport_policy, FirstPartyOnly);
}

fn trigger() -> TriggerPattern {
    TriggerPattern {
        kind: TriggerKind::MotivationDip,
        sport: Some("ride".to_owned()),
        magnitude: Band::Moderate,
    }
}

fn intervention() -> Intervention {
    Intervention {
        kind: InterventionKind::MinimumViable,
        magnitude: None,
    }
}

fn metric() -> OutcomeMetric {
    OutcomeMetric::ActivityCompleted {
        window_days: 2,
        sport: Some("ride".to_owned()),
    }
}

#[tokio::test]
async fn advice_and_playbooks_keep_their_stamp_and_stay_out_of_the_priors() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (trigger, intervention, metric) = (trigger(), intervention(), metric());
    let now = Utc::now();

    repos
        .playbooks
        .insert_pending_advice(&PendingAdvice {
            id: Uuid::new_v4().to_string(),
            tenant_id: "t1".to_owned(),
            user_id: "u1".to_owned(),
            agent_slug: None,
            playbook_id: None,
            trigger: trigger.clone(),
            intervention: intervention.clone(),
            outcome_metric: metric.clone(),
            baseline: MetricBaseline { captured_at: now },
            due_by: now - Duration::days(1),
            status: AdviceStatus::Pending,
            label: None,
            label_source: None,
            source_msg_id: Some("msg-1".to_owned()),
            created_at: now - Duration::days(3),
            transport_policy: FirstPartyOnly,
        })
        .await
        .unwrap();
    let due = repos
        .playbooks
        .due_pending_advice(now.timestamp(), 10)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].transport_policy, FirstPartyOnly);

    let outcome = |user: &'static str, policy| RecordedOutcome {
        tenant_id: "t1",
        user_id: user,
        agent_slug: None,
        trigger: &trigger,
        intervention: &intervention,
        outcome_metric: &metric,
        label: OutcomeLabel::Success,
        at: Utc::now(),
        transport_policy: policy,
    };
    let label = |user: &'static str, policy| {
        let repos = Arc::clone(&repos);
        let recorded = outcome(user, policy);
        async move {
            repos
                .playbooks
                .record_outcome_and_label(&recorded, "advice", LabelSource::DataHeuristic)
                .await
                .unwrap()
        }
    };
    label("u1", AnyTransport).await;
    let open = repos
        .playbooks
        .list_all_user_playbooks("t1", "u1", 10, TransportPolicy::FirstPartyOnly)
        .await
        .unwrap();
    assert_eq!(open[0].transport_policy, AnyTransport);
    label("u1", FirstPartyOnly).await;
    label("u1", AnyTransport).await;
    let tightened = repos
        .playbooks
        .list_all_user_playbooks("t1", "u1", 10, TransportPolicy::FirstPartyOnly)
        .await
        .unwrap();
    assert_eq!(tightened.len(), 1, "the outcomes fold into one playbook");
    assert_eq!(
        tightened[0].transport_policy, FirstPartyOnly,
        "an outcome read off first-party-only data keeps the playbook there"
    );

    label("u2", AnyTransport).await;
    let aggregated = repos.playbooks.aggregate_playbook_rows(100).await.unwrap();
    let users: Vec<&str> = aggregated.iter().map(|row| row.user_id.as_str()).collect();
    assert_eq!(
        users,
        vec!["u2"],
        "a stamped playbook never feeds priors served to other athletes"
    );
}

#[tokio::test]
async fn verdicts_profiles_and_prescriptions_keep_their_stamp() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    let user = user_id.to_string();
    let conversation = repos
        .chat
        .create_conversation(&user, tenant, "Verdicts", "model", None, None)
        .await
        .unwrap();
    let reply = repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: tenant,
            conversation_id: &conversation.id,
            user_id: &user,
            role: "assistant",
            content: "your climbing improved on the relay's loop",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: FirstPartyOnly,
        })
        .await
        .unwrap();

    repos
        .claim_verdicts
        .insert_claim_verdict(&InsertClaimVerdictParams {
            tenant_id: tenant,
            user_id: &user,
            agent_id: None,
            conversation_id: Some(&conversation.id),
            message_id: Some(&reply.id),
            claim_text: "your climbing improved on the relay's loop",
            category: ClaimCategory::TrainingPrescription,
            status: ClaimStatus::Supported,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.8,
            layer_fired: VerdictLayer::Evidence,
            explanation: None,
            evidence_refs: None,
            transport_policy: FirstPartyOnly,
        })
        .await
        .unwrap();
    let verdicts = repos
        .claim_verdicts
        .list_verdicts_for_conversation(&conversation.id, tenant)
        .await
        .unwrap();
    assert_eq!(verdicts[0].transport_policy, FirstPartyOnly);

    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Ride);
    profile.ftp_watts = Some(280);
    profile.fitness_level = FitnessLevel::Advanced;
    profile.transport_policy = FirstPartyOnly;
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await
        .unwrap();
    let stored = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.transport_policy, FirstPartyOnly);
    let withheld = repos
        .dossier
        .compose_dossier(tenant, user_id, TransportPolicy::AnyTransport, &|policy| {
            !policy.is_first_party_only()
        })
        .await
        .unwrap();
    assert!(
        withheld.physiology.is_none() && withheld.power_zones.is_none(),
        "a caller that may not read the stamp gets no profile and nothing derived from it"
    );
    let served = repos
        .dossier
        .compose_dossier(tenant, user_id, TransportPolicy::FirstPartyOnly, &|_| true)
        .await
        .unwrap();
    assert_eq!(served.physiology.and_then(|p| p.ftp_watts), Some(280));

    let date = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
    let row = |policy| PrescribedWorkout {
        id: Uuid::nil(),
        tenant_id: tenant.as_uuid(),
        user_id,
        agent_id: None,
        template_slug: Some("threshold-2x20".to_owned()),
        sport: SportType::Ride,
        prescribed_for_date: date,
        provider: "intervals_icu".to_owned(),
        provider_event_id: Some("evt-1".to_owned()),
        external_id: Some("key-1".to_owned()),
        source: CalendarEventSource::Prescription,
        plan_week_id: None,
        replaces_id: None,
        payload_hash: Some("h1".to_owned()),
        payload_json: "{}".to_owned(),
        status: PrescribedWorkout::STATUS_PUSHED.to_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        transport_policy: policy,
    };
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&row(FirstPartyOnly))
        .await
        .unwrap();
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&row(AnyTransport))
        .await
        .unwrap();
    let ledger = repos
        .prescribed_workouts
        .get_prescribed_workout(tenant, user_id, Uuid::nil())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ledger.transport_policy, FirstPartyOnly,
        "refreshing an entry never loosens its stamp"
    );
}

/// One week a weeks-only save adds to the season.
static WEEK: [PlanWeekInput<'static>; 1] = [PlanWeekInput {
    week_start: "2026-11-02",
    focus: "threshold",
    days: &[],
    adjustment_reason: "",
    phase_index: None,
}];

fn goal() -> GoalRace {
    GoalRace {
        name: "Autumn Fondo".to_owned(),
        date: "2026-12-06".to_owned(),
        discipline: "road".to_owned(),
        priority: RacePriority::A,
    }
}

#[tokio::test]
async fn a_plan_is_as_strict_as_everything_saved_onto_it() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user_id, tenant): (Uuid, TenantId) = seed_user(&db).await;
    let (tenant_id, user) = (tenant.to_string(), user_id.to_string());
    let goal = goal();
    let save = |outline: bool, weeks: &'static [PlanWeekInput<'static>], policy| {
        let repos = Arc::clone(&repos);
        let (tenant_id, user, goal) = (tenant_id.clone(), user.clone(), goal.clone());
        async move {
            repos
                .training_plans
                .save_plan_bundle(&SavePlanBundleParams {
                    tenant_id: &tenant_id,
                    user_id: &user,
                    author: PlanAuthor::none(),
                    goal_fact_id: None,
                    replace_season: false,
                    outline: outline.then_some(PlanOutlineInput {
                        goal_race: &goal,
                        races: Some(&[]),
                        strategy: "build then sharpen",
                        phases: &[],
                        source_conversation_id: None,
                        flavour: None,
                        season_start: None,
                        season_end: None,
                    }),
                    weeks,
                    transport_policy: policy,
                })
                .await
                .unwrap()
                .plan
                .transport_policy
        }
    };
    let active = || {
        let repos = Arc::clone(&repos);
        let (tenant_id, user) = (tenant_id.clone(), user.clone());
        async move {
            repos
                .training_plans
                .get_active_plan(&tenant_id, &user)
                .await
                .unwrap()
                .unwrap()
                .transport_policy
        }
    };

    assert_eq!(save(true, &[], AnyTransport).await, AnyTransport);
    assert_eq!(active().await, AnyTransport);

    assert_eq!(
        save(false, &WEEK, FirstPartyOnly).await,
        FirstPartyOnly,
        "a week written from first-party-only data tightens the plan it lands on"
    );
    assert_eq!(active().await, FirstPartyOnly);

    assert_eq!(
        save(true, &[], AnyTransport).await,
        FirstPartyOnly,
        "a new outline carries the stamped season's weeks, and its stamp"
    );
    assert_eq!(active().await, FirstPartyOnly);
}

/// Stamped rows written after the open ones fill the newest page; a reader
/// that may not read them still gets `limit` open rows, because the stamp is
/// filtered in the statement, before its `LIMIT`.
#[tokio::test]
async fn a_limit_counts_only_the_rows_the_reader_may_read() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    let user = user_id.to_string();
    let pause = || sleep(StdDuration::from_millis(5));

    let conversation = repos
        .chat
        .create_conversation(&user, tenant, "Fill", "model", None, None)
        .await
        .unwrap();
    for (prefix, policy) in [("open", AnyTransport), ("stamped", FirstPartyOnly)] {
        for n in 0..3 {
            let content = format!("{prefix} {n}");
            repos
                .chat
                .add_message(&AddMessageParams {
                    tenant_id: tenant,
                    conversation_id: &conversation.id,
                    user_id: &user,
                    role: "assistant",
                    content: &content,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                    transport_policy: policy,
                })
                .await
                .unwrap();
            repos
                .memory
                .upsert_user_fact(&UpsertUserFactParams {
                    tenant_id: tenant,
                    user_id: &user,
                    agent_id: None,
                    scope: MemoryScope::User,
                    kind: FactKind::Preference,
                    pillar: None,
                    predicate_code: PredicateCode::Prefer,
                    object: &content,
                    confidence: 0.8,
                    source: FactSource::Onboarding,
                    valid_until: None,
                    source_msg_id: None,
                    transport_policy: policy,
                })
                .await
                .unwrap();
            pause().await;
        }
    }

    let open_only = |rows: Vec<String>| rows.iter().all(|r| r.starts_with("open"));
    let recent = repos
        .chat
        .get_recent_messages(&conversation.id, &user, tenant, 2, AnyTransport)
        .await
        .unwrap();
    let recent: Vec<String> = recent.into_iter().map(|m| m.content).collect();
    assert_eq!(recent.len(), 2);
    assert!(open_only(recent.clone()), "{recent:?}");

    let facts = repos
        .memory
        .list_user_facts(tenant, &user, None, None, 2, AnyTransport)
        .await
        .unwrap();
    let facts: Vec<String> = facts.into_iter().map(|f| f.object).collect();
    assert_eq!(facts.len(), 2);
    assert!(open_only(facts.clone()), "{facts:?}");
    let by_source = repos
        .memory
        .list_user_facts_by_source(tenant, &user, FactSource::Onboarding, 2, AnyTransport)
        .await
        .unwrap();
    assert_eq!(by_source.len(), 2);
    assert!(by_source.iter().all(|f| f.transport_policy == AnyTransport));
    let first_party = repos
        .memory
        .list_user_facts(tenant, &user, None, None, 2, FirstPartyOnly)
        .await
        .unwrap();
    assert!(
        first_party
            .iter()
            .all(|f| f.transport_policy == FirstPartyOnly),
        "the app reads the newest rows, stamped or not"
    );

    let page = repos
        .chat
        .list_conversations(&user, tenant, 10, 0, AnyTransport)
        .await
        .unwrap();
    let row = &page.items[0];
    assert_eq!(row.message_count, 3, "only readable rows are counted");
    assert!(
        row.last_message
            .as_ref()
            .is_some_and(|m| m.content_head.starts_with("open")),
        "the preview is the newest readable row"
    );
}
