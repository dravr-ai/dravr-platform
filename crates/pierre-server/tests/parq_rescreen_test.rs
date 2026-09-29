// ABOUTME: A clean PAR-Q re-screen retires the flag its own "yes" raised — and nothing else
// ABOUTME: REST POST /api/me/parq and the messaging intake's answer recorder; coach-raised flags always stand
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every reader of a medical flag reads it whatever its freshness horizon, so
//! a PAR-Q "yes" nothing retired would withhold nutrition figures forever. A
//! "no" on a re-screen is the athlete's answer now: it retires the flag the
//! earlier "yes" to the same question raised. A question the re-screen does
//! not answer keeps its flag, and a medical fact a coach tool raised is never
//! touched by a screen.

mod common;
mod helpers;

use std::sync::Arc;

use anyhow::Result;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use pierre_core::models::TenantId;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::intake::{record_parq_no, record_parq_yes, IntakeTopic};
use pierre_services::medical_flag::{nutrition_figures_gate, FiguresWithheld};
use pierre_services::parq::{self, PARQ_QUESTION_IDS};
use serde_json::{json, Value};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;

struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

impl Athlete {
    async fn new() -> Result<Self> {
        common::init_server_config();
        let resources = create_test_server_resources().await?;
        let (user_id, user) = create_test_user(&resources.agent.database).await?;
        let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
        let tenant = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await?
            .first()
            .map(|t| t.id)
            .expect("the athlete has a tenant");
        Ok(Self {
            resources,
            user_id,
            tenant,
            token,
        })
    }

    /// Submit a PAR-Q screen over REST: `(question, yes)` pairs.
    async fn screen(&self, answers: &[(&str, bool)]) -> Value {
        let answers: Vec<Value> = answers
            .iter()
            .map(|(id, yes)| json!({ "id": id, "yes": yes }))
            .collect();
        let resp = AxumTestRequest::post("/api/me/parq")
            .header("Authorization", &self.token)
            .json(&json!({ "answers": answers }))
            .send(OnboardingRoutes::routes(Arc::clone(&self.resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        resp.json()
    }

    /// A clean answer to every one of the seven questions.
    async fn clean_screen(&self) -> Value {
        let answers: Vec<(&str, bool)> = PARQ_QUESTION_IDS.iter().map(|id| (*id, false)).collect();
        self.screen(&answers).await
    }

    /// The medical facts on file, as `(predicate_code, object, source)`.
    async fn medical(&self) -> Result<Vec<(String, String, String)>> {
        let dossier = self
            .resources
            .common
            .repos
            .dossier
            .compose_dossier(self.tenant, self.user_id)
            .await?;
        let mut facts: Vec<_> = dossier
            .medical
            .iter()
            .map(|f| (f.predicate_code.clone(), f.object.clone(), f.source.clone()))
            .collect();
        facts.sort();
        Ok(facts)
    }

    /// The ids of the medical facts on file.
    async fn medical_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .resources
            .common
            .repos
            .memory
            .list_user_facts(
                self.tenant,
                &self.user_id.to_string(),
                None,
                Some(FactKind::Medical),
                10,
            )
            .await?
            .into_iter()
            .map(|fact| fact.id)
            .collect())
    }

    async fn gate(&self) -> Result<Option<FiguresWithheld>> {
        Ok(nutrition_figures_gate(
            self.resources.common.repos.as_ref(),
            Some(self.tenant),
            self.user_id,
        )
        .await?)
    }

    /// A medical flag a coach tool raised.
    async fn coach_flag(&self, object: &str) -> Result<()> {
        self.resources
            .common
            .repos
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id: self.tenant,
                user_id: &self.user_id.to_string(),
                agent_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Medical,
                pillar: None,
                predicate_code: PredicateCode::Flagged,
                object,
                confidence: 0.9,
                source: FactSource::Coach,
                valid_until: Some(Utc::now() + Duration::days(90)),
                source_msg_id: None,
            })
            .await?;
        Ok(())
    }
}

fn parq_flag(question: &str) -> (String, String, String) {
    (
        "parq_yes".to_owned(),
        question.to_owned(),
        "onboarding".to_owned(),
    )
}

// ============================================================================
// REST: POST /api/me/parq
// ============================================================================

#[tokio::test]
async fn a_clean_rescreen_lifts_the_gate_a_parq_yes_raised() -> Result<()> {
    let athlete = Athlete::new().await?;
    let first = athlete.screen(&[("heart_condition", true)]).await;
    assert_eq!(first["flags_raised"], 1);
    assert_eq!(athlete.medical().await?, vec![parq_flag("heart_condition")]);
    assert_eq!(athlete.gate().await?, Some(FiguresWithheld::medical_flag()));

    let rescreen = athlete.clean_screen().await;
    assert_eq!(rescreen["flags_raised"], 0);
    assert!(
        athlete.medical().await?.is_empty(),
        "the clean answer retires the flag its earlier yes raised"
    );
    assert_eq!(
        athlete.gate().await?,
        None,
        "with no flag left the nutrition figures are released"
    );
    Ok(())
}

#[tokio::test]
async fn a_coach_raised_flag_still_gates_after_a_clean_rescreen() -> Result<()> {
    let athlete = Athlete::new().await?;
    athlete.coach_flag("type 1 diabetes").await?;
    athlete.screen(&[("chest_pain", true)]).await;

    athlete.clean_screen().await;
    assert_eq!(
        athlete.medical().await?,
        vec![(
            "flagged".to_owned(),
            "type 1 diabetes".to_owned(),
            "coach".to_owned()
        )],
        "a screen retires only the flags a screen raised"
    );
    assert_eq!(
        athlete.gate().await?,
        Some(FiguresWithheld::medical_flag()),
        "the coach's flag still withholds the figures"
    );
    Ok(())
}

#[tokio::test]
async fn a_partial_rescreen_retires_only_the_questions_it_cleared() -> Result<()> {
    let athlete = Athlete::new().await?;
    athlete
        .screen(&[
            ("heart_condition", true),
            ("chest_pain", true),
            ("joint_problem", true),
        ])
        .await;

    // Only chest pain is re-asked, and answered "no"; the other two are not
    // part of this submission and keep what is on file.
    athlete.screen(&[("chest_pain", false)]).await;
    assert_eq!(
        athlete.medical().await?,
        vec![parq_flag("heart_condition"), parq_flag("joint_problem")]
    );
    assert_eq!(athlete.gate().await?, Some(FiguresWithheld::medical_flag()));

    // A mixed re-screen: one question cleared, one confirmed.
    athlete
        .screen(&[("heart_condition", false), ("joint_problem", true)])
        .await;
    assert_eq!(athlete.medical().await?, vec![parq_flag("joint_problem")]);
    Ok(())
}

#[tokio::test]
async fn a_repeated_yes_refreshes_its_flag_instead_of_stacking_a_second() -> Result<()> {
    let athlete = Athlete::new().await?;
    athlete.screen(&[("dizziness", true)]).await;
    let first = athlete.medical_ids().await?;
    athlete.screen(&[("dizziness", true)]).await;
    assert_eq!(
        athlete.medical().await?,
        vec![parq_flag("dizziness")],
        "one question, one flag"
    );
    // The flag that stays is the one the second "yes" wrote: it is written
    // before the old one is deleted, so the athlete is never without it.
    let second = athlete.medical_ids().await?;
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(second, first, "the repeated yes replaced the earlier flag");

    // One "no" is then enough to retire it.
    athlete.screen(&[("dizziness", false)]).await;
    assert!(athlete.medical().await?.is_empty());
    Ok(())
}

// ============================================================================
// The messaging intake's answer recorder
// ============================================================================

#[tokio::test]
async fn a_clean_chat_answer_retires_only_its_own_questions_flag() -> Result<()> {
    let athlete = Athlete::new().await?;
    let memory = athlete.resources.common.repos.memory.as_ref();
    let user = athlete.user_id.to_string();
    athlete.coach_flag("recent surgery").await?;
    record_parq_yes(memory, athlete.tenant, &user, IntakeTopic::ChestPain).await?;
    record_parq_yes(memory, athlete.tenant, &user, IntakeTopic::Medication).await?;

    let retired = record_parq_no(memory, athlete.tenant, &user, IntakeTopic::ChestPain).await?;
    assert_eq!(retired, 1);
    assert_eq!(
        athlete.medical().await?,
        vec![
            (
                "flagged".to_owned(),
                "recent surgery".to_owned(),
                "coach".to_owned()
            ),
            parq_flag("medication"),
        ]
    );

    // The profile-type topic screens nothing, and a "no" to a question with
    // no flag on file retires nothing.
    assert_eq!(
        record_parq_no(memory, athlete.tenant, &user, IntakeTopic::Persona).await?,
        0
    );
    assert_eq!(
        record_parq_no(memory, athlete.tenant, &user, IntakeTopic::Dizziness).await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn retiring_ignores_ids_outside_the_instrument() -> Result<()> {
    let athlete = Athlete::new().await?;
    let memory = athlete.resources.common.repos.memory.as_ref();
    let user = athlete.user_id.to_string();
    athlete.coach_flag("not_a_question").await?;

    let retired = parq::retire_parq_flags(
        memory,
        athlete.tenant,
        &user,
        &["not_a_question".to_owned()],
    )
    .await?;
    assert_eq!(retired, 0);
    assert_eq!(athlete.medical().await?.len(), 1);
    Ok(())
}
