// ABOUTME: The one-time in-app question asking an athlete to let Dravr's own agent read its memory (T3) and their health records (T2)
// ABOUTME: Due once per athlete until answered; an answer is the sensitive_t3 / sensitive_t2 consent itself, a dismissal withholds both

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The agent's tier consent, asked once (carnet#1544).
//!
//! JF 2026-11-25: Dravr's own agent reads its memory and the athlete's notes
//! only under `sensitive_t3`, and their health records (PAR-Q answers,
//! injuries) only under `sensitive_t2`, both withheld by default (Law 25
//! opt-in). Every athlete is asked once, in Dravr's app, whether to allow
//! them. The answer is recorded as the consents themselves
//! ([`provider_notice::grant_consent`]); a dismissal gives neither and the question is not
//! asked again. Either way the switches stay in the privacy settings.
//!
//! That the question was answered is the `agent_consent_prompt` step row in
//! `user_onboarding`, keyed by user like every other step: `complete` for an
//! answer, `skipped` for a dismissal. No other step reader matches it.

use pierre_core::constants::oauth::providers::{
    sensitive_display_name, SENSITIVE_T2, SENSITIVE_T3,
};
use pierre_core::constants::oauth_providers::NoticeKind;
use pierre_core::errors::AppResult;
use pierre_database::RepositoryRegistry;
use serde::Serialize;
use uuid::Uuid;

use crate::intake::{STATUS_COMPLETE, STATUS_SKIPPED};
use crate::provider_notice;

/// The `user_onboarding` step that records the question was answered.
pub const STEP_AGENT_CONSENT_PROMPT: &str = "agent_consent_prompt";

/// What the app shows for the question, and the athlete's standing answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AgentConsentPrompt {
    /// Ask now: not answered yet, and at least one of the two consents is
    /// asked of this account and not given.
    pub due: bool,
    /// `sensitive_t3` is given: the agent reads its memory and the notes.
    pub memory: bool,
    /// `sensitive_t2` is given: the agent reads the health records.
    pub medical: bool,
}

/// The athlete's answer to the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentConsentAnswer {
    /// Saved: give each consent ticked. One left unticked is not withdrawn.
    Allow {
        /// Give `sensitive_t3`.
        memory: bool,
        /// Give `sensitive_t2`.
        medical: bool,
    },
    /// Dismissed ("Not now"): give neither, and do not ask again.
    Dismiss,
}

/// The question's state for `user_id` in `tenant_id`.
///
/// # Errors
///
/// Returns the repository error when the step rows cannot be read.
pub async fn agent_consent_prompt(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
) -> AppResult<AgentConsentPrompt> {
    let memory = provider_notice::consent_state(
        repos,
        tenant_id,
        user_id,
        SENSITIVE_T3,
        NoticeKind::AiConsent,
    )
    .await;
    let medical = provider_notice::consent_state(
        repos,
        tenant_id,
        user_id,
        SENSITIVE_T2,
        NoticeKind::AiConsent,
    )
    .await;
    let answered = repos
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await?
        .iter()
        .any(|step| step.step_id == STEP_AGENT_CONSENT_PROMPT);
    let outstanding = memory == Some(false) || medical == Some(false);
    Ok(AgentConsentPrompt {
        due: !answered && outstanding,
        memory: memory == Some(true),
        medical: medical == Some(true),
    })
}

/// Record the athlete's `answer` and return the question's new state.
///
/// An allowed consent is given in its notice's current version, exactly as
/// the settings switch gives it; nothing already given is withdrawn, by an
/// unticked box or a dismissal. The question is then recorded as answered,
/// so it is not asked again.
///
/// # Errors
///
/// Returns the store's error when a consent or the step cannot be written.
pub async fn answer_agent_consent_prompt(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    answer: AgentConsentAnswer,
) -> AppResult<AgentConsentPrompt> {
    let status = match answer {
        AgentConsentAnswer::Allow { memory, medical } => {
            for (class, given) in [(SENSITIVE_T3, memory), (SENSITIVE_T2, medical)] {
                if given {
                    let brand = sensitive_display_name(class).unwrap_or(class);
                    provider_notice::grant_consent(
                        repos,
                        tenant_id,
                        user_id,
                        class,
                        NoticeKind::AiConsent,
                        brand,
                    )
                    .await?;
                }
            }
            STATUS_COMPLETE
        }
        AgentConsentAnswer::Dismiss => STATUS_SKIPPED,
    };
    let tenant = tenant_id.to_string();
    repos
        .user_onboarding
        .set_onboarding_step(
            &user_id.to_string(),
            STEP_AGENT_CONSENT_PROMPT,
            status,
            None,
            Some(&tenant),
        )
        .await?;
    agent_consent_prompt(repos, tenant_id, user_id).await
}
