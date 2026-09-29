// ABOUTME: The athlete's medical/PAR-Q flag read, and the statement a surface carries when it withholds figures for it
// ABOUTME: One reader for the nutrition tools and every plan fuelling surface, so one flag withholds amounts everywhere
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Medical flag
//!
//! A medical flag on the athlete's dossier means a clinician sets their
//! nutrition amounts. Two families of surface honour it: the tools that
//! *prescribe* an amount (`calculate_daily_nutrition` and the rest of the
//! nutrition gate in the tool runtime), and every surface that *shows* the
//! per-session fuelling a plan stores (`get_training_plan`, the plan card, the
//! prompt's plan block, the calendar push — see [`crate::plan_fueling`]).
//!
//! Both read the flag here and carry the same [`FiguresWithheld`] statement,
//! so an athlete's clinician is named the same way on every surface, and a
//! read that fails never releases a figure the flag would have withheld.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::FactKind;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Why an answer came back without its nutrition figures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WithheldReason {
    /// A medical / PAR-Q flag is on the athlete's dossier.
    MedicalFlag,
}

/// Who sets the withheld figures instead of the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FigureAuthority {
    /// The athlete's clinician.
    Clinician,
}

/// The machine-readable statement that figures were withheld, and why.
///
/// Present means the athlete has a medical flag on file and the answer carries
/// no amount to eat or drink: no energy, macronutrient, fluid, sodium or
/// caffeine intake. What a finished session cost (its calories burned) still
/// describes the session and is not an intake. Relay `note` rather than
/// supplying a number of your own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FiguresWithheld {
    /// `medical_flag`: a medical / PAR-Q flag is on file.
    pub reason: WithheldReason,
    /// `clinician`: who sets these amounts for this athlete.
    pub set_by: FigureAuthority,
    /// What to tell the athlete, in plain words.
    pub note: String,
}

/// The note every withholding surface carries, so the agent says one thing
/// everywhere.
const MEDICAL_FLAG_NOTE: &str = "A medical/PAR-Q flag is on file for this athlete, so their \
     energy, protein, carbohydrate, fat, fluid, sodium and caffeine amounts are set by their \
     clinician and were withheld here. Ask what their clinician has advised and work inside it.";

impl FiguresWithheld {
    /// The statement for an athlete with a medical flag on file.
    #[must_use]
    pub fn medical_flag() -> Self {
        Self {
            reason: WithheldReason::MedicalFlag,
            set_by: FigureAuthority::Clinician,
            note: MEDICAL_FLAG_NOTE.to_owned(),
        }
    }
}

/// The tenants whose facts describe this athlete for this call.
///
/// The call's own tenant when it carries one — the same scope the dossier and
/// the agent prompt read. A call with no tenant (an MCP client whose token
/// names none) reads every tenant the athlete belongs to, because skipping the
/// read would hand that client the figures the gate exists to withhold.
///
/// # Errors
///
/// Returns the repository error when the athlete's tenants cannot be listed.
pub async fn athlete_tenants(
    repos: &RepositoryRegistry,
    tenant_id: Option<TenantId>,
    user_id: Uuid,
) -> AppResult<Vec<TenantId>> {
    match tenant_id {
        Some(tenant) => Ok(vec![tenant]),
        None => Ok(repos
            .tenants
            .list_for_user(user_id)
            .await?
            .into_iter()
            .map(|tenant| tenant.id)
            .collect()),
    }
}

/// Whether a medical flag is on file for the athlete under any of `tenants`.
///
/// A flag is any `FactKind::Medical` fact: a PAR-Q "yes" (`parq_yes`) or one a
/// coach tool raised (`flagged`). That is exactly what the dossier's `medical`
/// bucket holds — `group_facts` routes every medical fact there — and this is
/// the same by-kind read `compose_dossier` makes to guarantee them a place.
/// A stale flag counts: the PAR-Q horizon asks for a re-screen, and the answer
/// on file stands until the flag itself is gone. A PAR-Q flag goes when a
/// re-screen answers its question "no" (`parq::retire_parq_flags`); any flag
/// goes when its fact is deleted (the athlete's memory Forget). A coach tool's
/// flag is never retired by a screen.
///
/// Read errors propagate rather than reading as "no flag": the dossier degrades
/// a failed read to an empty set so a prompt still renders, but here that would
/// release the figures on a flag nobody could read.
///
/// # Errors
///
/// Returns the repository error when a fact read fails.
pub async fn medical_flag_on_file(
    repos: &RepositoryRegistry,
    tenants: &[TenantId],
    user_id: Uuid,
) -> AppResult<bool> {
    let user = user_id.to_string();
    for tenant in tenants {
        let flags = repos
            .memory
            .list_user_facts(*tenant, &user, None, Some(FactKind::Medical), 1)
            .await?;
        if !flags.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The gate: `Some` when the athlete's nutrition figures must be withheld,
/// `None` when a surface may show them.
///
/// # Errors
///
/// Returns the repository error when the tenants or the flag cannot be read.
pub async fn nutrition_figures_gate(
    repos: &RepositoryRegistry,
    tenant_id: Option<TenantId>,
    user_id: Uuid,
) -> AppResult<Option<FiguresWithheld>> {
    let tenants = athlete_tenants(repos, tenant_id, user_id).await?;
    Ok(medical_flag_on_file(repos, &tenants, user_id)
        .await?
        .then(FiguresWithheld::medical_flag))
}
