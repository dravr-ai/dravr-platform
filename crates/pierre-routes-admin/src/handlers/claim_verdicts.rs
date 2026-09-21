// ABOUTME: Admin routes for triaging claim verdicts from the bullshit detector
// ABOUTME: List/filter, detail with the knob to adjust, support disposition, message lookup, aggregate health
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin claim verdict triage routes.
//!
//! Surfaces the rows written by the detector pipeline
//! (`services::claim_verification::apply_claim_verification`) so tenant
//! admins can review flagged claims, see which agent emitted them, drill
//! into the supporting evidence, and record whether the flag was right.
//!
//! The detail read carries a [`VerdictKnob`]: the one place in this repo or
//! in dravr-contremaitre that decides what the verdict's layer does, computed
//! here from the layer and the row so the web client does no guessing.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;

use pierre_contremaitre::EvidenceRegistry;
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    DispositionFilter, SetVerdictDispositionParams, VerdictListFilter, UNDISPOSED_FILTER,
};
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, ClaimVerdict, DispositionReason, VerdictDisposition, VerdictLayer,
};

use crate::context::AdminApiContext;

/// Longest disposition note accepted, in characters. A triage note is a
/// sentence or two; anything longer belongs in the issue it should link to.
pub const MAX_DISPOSITION_NOTE_CHARS: usize = 2000;

/// How many matching propositions the evidence knob names. Retrieval itself
/// considers the top three; a couple more shows what would have matched with
/// one more keyword.
const KNOB_PROPOSITION_LIMIT: usize = 5;

/// Query parameters for listing claim verdicts.
#[derive(Debug, Deserialize)]
pub struct ListVerdictsQuery {
    /// Tenant to list verdicts for (required — admin tokens can span tenants).
    pub tenant_id: String,
    /// Optional filter by verdict status.
    pub status: Option<String>,
    /// Optional filter by claim category.
    pub category: Option<String>,
    /// Optional filter by the agent that emitted the claim.
    pub agent_id: Option<String>,
    /// Optional filter by the layer that decided the verdict.
    pub layer_fired: Option<String>,
    /// Optional filter by disposition: a disposition value, or `undisposed`
    /// for the triage queue.
    pub disposition: Option<String>,
    /// Optional filter by the athlete the claim was said to.
    pub user_id: Option<String>,
    /// Maximum number of rows to return, clamped to `1..=200`. Defaults to 50.
    pub limit: Option<i64>,
}

/// Query parameters for a read scoped to one tenant.
#[derive(Debug, Deserialize)]
pub struct TenantScopedQuery {
    /// Tenant that owns the rows.
    pub tenant_id: String,
}

/// Query parameters for the aggregate health read.
#[derive(Debug, Deserialize)]
pub struct VerdictHealthQuery {
    /// Tenant to aggregate.
    pub tenant_id: String,
    /// Window in days, clamped to `1..=365`. Defaults to 30.
    pub window_days: Option<i64>,
}

/// Request body for `PUT /api/admin/claim-verdicts/{id}/disposition`.
#[derive(Debug, Deserialize)]
pub struct SetDispositionRequest {
    /// Tenant that owns the verdict.
    pub tenant_id: String,
    /// `true_catch`, `false_positive` or `unsure`.
    pub disposition: String,
    /// One of the [`DispositionReason`] strings, when the triager named one.
    pub reason: Option<String>,
    /// Free-text note, at most [`MAX_DISPOSITION_NOTE_CHARS`] characters.
    pub note: Option<String>,
}

/// Wire-format representation of a `ClaimVerdict` for the admin UI.
///
/// Flat strings everywhere so the TypeScript client can render without
/// mapping `snake_case` enums into display labels on the fly.
#[derive(Debug, Serialize)]
pub struct VerdictRow {
    /// Stable identifier for this verdict row.
    pub id: String,
    /// Tenant that owns the underlying conversation.
    pub tenant_id: String,
    /// User the verdict was issued for.
    pub user_id: String,
    /// Agent that produced the claim, when known.
    pub agent_id: Option<String>,
    /// Conversation the claim came from, when known.
    pub conversation_id: Option<String>,
    /// Message within the conversation the claim came from, when known.
    pub message_id: Option<String>,
    /// Raw claim text extracted from the agent response.
    pub claim_text: String,
    /// Domain category of the claim (training, nutrition, recovery, ...).
    pub category: String,
    /// Verdict status: `supported`, `unsupported`, `contradicted`,
    /// `rhetorical` or `unverifiable`.
    pub status: String,
    /// Strength of the evidence backing the verdict (`strong`, `weak`, ...).
    pub evidence_strength: String,
    /// Model confidence in the verdict in [0.0, 1.0].
    pub confidence: f32,
    /// Layer of the myth-busting pipeline that produced this verdict.
    pub layer_fired: String,
    /// Free-form explanation of the verdict, when available.
    pub explanation: Option<String>,
    /// Comma-separated evidence record ids (DOIs / PMIDs), when available.
    pub evidence_refs: Option<String>,
    /// RFC3339 timestamp the verdict was recorded.
    pub created_at: String,
    /// Support's judgement, once someone has read the verdict.
    pub disposition: Option<String>,
    /// The pipeline input the triager blamed, when they named one.
    pub disposition_reason: Option<String>,
    /// Free-text note left with the disposition.
    pub disposition_note: Option<String>,
    /// Who disposed it.
    pub disposed_by: Option<String>,
    /// RFC3339 timestamp of the disposition.
    pub disposed_at: Option<String>,
}

impl From<ClaimVerdict> for VerdictRow {
    fn from(v: ClaimVerdict) -> Self {
        Self {
            id: v.id,
            tenant_id: v.tenant_id,
            user_id: v.user_id,
            agent_id: v.agent_id,
            conversation_id: v.conversation_id,
            message_id: v.message_id,
            claim_text: v.claim_text,
            category: v.category.as_str().to_owned(),
            status: v.status.as_str().to_owned(),
            evidence_strength: v.evidence_strength.as_str().to_owned(),
            confidence: v.confidence,
            layer_fired: v.layer_fired.as_str().to_owned(),
            explanation: v.explanation,
            evidence_refs: v.evidence_refs,
            created_at: v.created_at.to_rfc3339(),
            disposition: v.disposition.map(|d| d.as_str().to_owned()),
            disposition_reason: v.disposition_reason.map(|r| r.as_str().to_owned()),
            disposition_note: v.disposition_note,
            disposed_by: v.disposed_by,
            disposed_at: v.disposed_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// List response envelope used by the admin UI.
#[derive(Debug, Serialize)]
pub struct VerdictListResponse {
    /// Verdict rows for this page.
    pub verdicts: Vec<VerdictRow>,
    /// Number of rows returned in `verdicts`.
    pub total: usize,
}

/// One corpus proposition the evidence knob names.
#[derive(Debug, Serialize)]
pub struct KnobProposition {
    /// The record id — the `id:` of the proposition's frontmatter, which is
    /// what `evidence_refs` carries.
    pub id: String,
    /// The proposition's category folder.
    pub category: String,
    /// The proposition's file stem.
    pub slug: String,
    /// Its path in dravr-contremaitre.
    pub path: String,
    /// The evidence strength its frontmatter declares.
    pub strength: String,
    /// Keyword-overlap score against the claim, as retrieval computes it.
    pub score: usize,
    /// Whether the verdict's `evidence_refs` names this record.
    pub cited: bool,
}

/// Where the input that produced a verdict lives, so a disposition can be
/// turned into an edit.
///
/// Every `location` is a path that exists in this repository or in
/// dravr-contremaitre; `detail` names the identifiers at that path to look at.
#[derive(Debug, Serialize)]
pub struct VerdictKnob {
    /// The layer that decided the verdict.
    pub layer: String,
    /// What kind of input the layer reads: `rhetoric_filter`,
    /// `deterministic_bounds`, `personalized_tolerance`,
    /// `athlete_data_record`, `evidence_corpus`, `consistency_check` or
    /// `judge_prompt`.
    pub kind: String,
    /// Repository path of that input.
    pub location: String,
    /// The identifiers at that path, and what the row says about them.
    pub detail: String,
    /// For the evidence layer, the corpus propositions that keyword-match the
    /// claim on the registry as it is now; empty for every other layer.
    pub propositions: Vec<KnobProposition>,
}

/// Detail response: the row and the knob behind it.
#[derive(Debug, Serialize)]
pub struct VerdictDetailResponse {
    /// The verdict.
    pub verdict: VerdictRow,
    /// What to adjust if the verdict was wrong.
    pub knob: VerdictKnob,
}

fn parse_tenant(raw: &str) -> AppResult<TenantId> {
    TenantId::parse_str(raw)
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {raw}")))
}

/// A blank or whitespace-only filter value means "no filter".
fn present(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

/// The stable strings of every variant, comma-separated, for a rejection
/// message. Read off the enum's own `ALL` so the message can never name a
/// vocabulary `parse` does not accept.
fn vocabulary<T: Copy>(all: &[T], as_str: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|v| as_str(*v))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The disposition filter's vocabulary: every disposition plus the sentinel
/// that selects the undisposed queue.
fn disposition_filter_vocabulary() -> String {
    format!(
        "{}, {UNDISPOSED_FILTER}",
        vocabulary(VerdictDisposition::ALL, VerdictDisposition::as_str)
    )
}

/// Parse one enum-valued query parameter, naming the parameter and the
/// vocabulary on a miss so a typo is a 400 rather than an empty page. The
/// vocabulary is built only on that path.
fn parse_filter<T>(
    name: &str,
    raw: Option<&str>,
    parse: fn(&str) -> Option<T>,
    vocabulary: impl FnOnce() -> String,
) -> AppResult<Option<T>> {
    present(raw)
        .map(|s| {
            parse(s).ok_or_else(|| {
                AppError::invalid_input(format!(
                    "{name} must be one of {}, got `{s}`",
                    vocabulary()
                ))
            })
        })
        .transpose()
}

/// Handle `GET /api/admin/claim-verdicts`.
///
/// Returns the tenant's most recent verdicts matching the query's filters,
/// newest first, at most 200. Every filter is applied in SQL by
/// `list_verdicts_filtered`; an unknown filter value is a 400.
///
/// # Errors
///
/// `AppError` on auth failure, an invalid tenant or filter value, or a
/// repository error.
pub async fn handle_list_claim_verdicts(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<ListVerdictsQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&params.tenant_id)?;

    let tenant = parse_tenant(&params.tenant_id)?;

    // The `agent_id` filter is purely client-supplied — validate the format
    // before scanning so the admin sees a 400 on a typo instead of a
    // silently-empty result. We accept the canonical UUID encoding only.
    let agent_id = present(params.agent_id.as_deref())
        .map(|raw| {
            Uuid::parse_str(raw).map(|_| raw.to_owned()).map_err(|_| {
                AppError::invalid_input(format!("agent_id must be a UUID, got `{raw}`"))
            })
        })
        .transpose()?;

    let filter = VerdictListFilter {
        status: parse_filter(
            "status",
            params.status.as_deref(),
            ClaimStatus::parse,
            || vocabulary(ClaimStatus::ALL, ClaimStatus::as_str),
        )?,
        category: parse_filter(
            "category",
            params.category.as_deref(),
            ClaimCategory::parse,
            || vocabulary(ClaimCategory::ALL, ClaimCategory::as_str),
        )?,
        agent_id,
        layer_fired: parse_filter(
            "layer_fired",
            params.layer_fired.as_deref(),
            VerdictLayer::parse,
            || vocabulary(VerdictLayer::ALL, VerdictLayer::as_str),
        )?,
        disposition: parse_filter(
            "disposition",
            params.disposition.as_deref(),
            DispositionFilter::parse,
            disposition_filter_vocabulary,
        )?,
        user_id: present(params.user_id.as_deref()).map(ToOwned::to_owned),
        limit: params.limit.unwrap_or(50).clamp(1, 200),
    };

    let verdicts = context
        .repos
        .claim_verdicts
        .list_verdicts_filtered(tenant, &filter)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list claim verdicts");
            AppError::internal(format!("Failed to list claim verdicts: {e}"))
        })?;

    let rows: Vec<VerdictRow> = verdicts.into_iter().map(Into::into).collect();
    let total = rows.len();
    info!(
        service = %admin_token.service_name,
        total,
        "admin listed claim verdicts"
    );

    Ok((
        StatusCode::OK,
        Json(VerdictListResponse {
            verdicts: rows,
            total,
        }),
    ))
}

/// Handle `GET /api/admin/claim-verdicts/conversations/{conversation_id}`.
///
/// Returns every verdict tied to a specific conversation in chronological
/// order. Used by the admin drawer to show the full verification history
/// behind a single flagged message.
///
/// # Errors
///
/// `AppError` on auth failure, an invalid tenant, or a repository error.
pub async fn handle_list_verdicts_by_conversation(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(conversation_id): Path<String>,
    Query(params): Query<TenantScopedQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&params.tenant_id)?;

    let tenant = parse_tenant(&params.tenant_id)?;

    let verdicts = context
        .repos
        .claim_verdicts
        .list_verdicts_for_conversation(&conversation_id, tenant)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for conversation");
            AppError::internal(format!("Failed to list verdicts for conversation: {e}"))
        })?;

    let rows: Vec<VerdictRow> = verdicts.into_iter().map(Into::into).collect();
    let total = rows.len();

    Ok((
        StatusCode::OK,
        Json(VerdictListResponse {
            verdicts: rows,
            total,
        }),
    ))
}

/// Handle `GET /api/admin/claim-verdicts/messages/{message_id}`.
///
/// Support's entry point: an athlete disputes a reply, support pastes the
/// message id, and every verdict extracted from that message comes back
/// oldest first.
///
/// # Errors
///
/// `AppError` on auth failure, an invalid tenant, or a repository error.
pub async fn handle_list_verdicts_by_message(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(message_id): Path<String>,
    Query(params): Query<TenantScopedQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&params.tenant_id)?;

    let tenant = parse_tenant(&params.tenant_id)?;

    let verdicts = context
        .repos
        .claim_verdicts
        .list_verdicts_for_message(tenant, &message_id)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for message");
            AppError::internal(format!("Failed to list verdicts for message: {e}"))
        })?;

    let rows: Vec<VerdictRow> = verdicts.into_iter().map(Into::into).collect();
    let total = rows.len();

    Ok((
        StatusCode::OK,
        Json(VerdictListResponse {
            verdicts: rows,
            total,
        }),
    ))
}

/// Handle `GET /api/admin/claim-verdicts/{verdict_id}`.
///
/// One verdict with the knob behind it.
///
/// # Errors
///
/// `AppError` on auth failure, an invalid tenant, a verdict the tenant does
/// not hold (404), or a repository error.
pub async fn handle_get_claim_verdict(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(verdict_id): Path<String>,
    Query(params): Query<TenantScopedQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&params.tenant_id)?;

    let tenant = parse_tenant(&params.tenant_id)?;

    let verdict = context
        .repos
        .claim_verdicts
        .get_verdict(tenant, &verdict_id)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to read claim verdict");
            AppError::internal(format!("Failed to read claim verdict: {e}"))
        })?
        .ok_or_else(|| AppError::not_found(format!("Claim verdict {verdict_id}")))?;

    let knob = knob_for(&verdict, &context.evidence_registry);
    Ok((
        StatusCode::OK,
        Json(VerdictDetailResponse {
            verdict: verdict.into(),
            knob,
        }),
    ))
}

/// Handle `PUT /api/admin/claim-verdicts/{verdict_id}/disposition`.
///
/// Records support's judgement on a verdict. A second call overwrites the
/// first. The admin's service name lands on the row as `disposed_by` and the
/// write is logged with it, the way every other admin config write records
/// who acted.
///
/// # Errors
///
/// `AppError` on auth failure, an invalid tenant, disposition, reason or
/// note, a verdict the tenant does not hold (404), or a repository error.
pub async fn handle_set_verdict_disposition(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(verdict_id): Path<String>,
    Json(body): Json<SetDispositionRequest>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    admin_token.require_tenant_access(&body.tenant_id)?;

    let tenant = parse_tenant(&body.tenant_id)?;
    let disposition = VerdictDisposition::parse(body.disposition.trim()).ok_or_else(|| {
        AppError::invalid_input(format!(
            "disposition must be one of {}, got `{}`",
            vocabulary(VerdictDisposition::ALL, VerdictDisposition::as_str),
            body.disposition
        ))
    })?;
    let reason = parse_filter(
        "reason",
        body.reason.as_deref(),
        DispositionReason::parse,
        || vocabulary(DispositionReason::ALL, DispositionReason::as_str),
    )?;
    let note = present(body.note.as_deref());
    if let Some(n) = note {
        if n.chars().count() > MAX_DISPOSITION_NOTE_CHARS {
            return Err(AppError::invalid_input(format!(
                "note must be at most {MAX_DISPOSITION_NOTE_CHARS} characters"
            )));
        }
    }

    let verdict = context
        .repos
        .claim_verdicts
        .set_verdict_disposition(&SetVerdictDispositionParams {
            tenant_id: tenant,
            verdict_id: &verdict_id,
            disposition,
            reason,
            note,
            disposed_by: &admin_token.service_name,
            disposed_at: Utc::now(),
        })
        .await
        .map_err(|e| {
            if e.code == ErrorCode::ResourceNotFound {
                e
            } else {
                error!(error = %e, verdict_id = %verdict_id, "failed to set verdict disposition");
                AppError::internal(format!("Failed to set verdict disposition: {e}"))
            }
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %body.tenant_id,
        verdict_id = %verdict_id,
        disposition = disposition.as_str(),
        reason = reason.map(DispositionReason::as_str),
        layer_fired = verdict.layer_fired.as_str(),
        "admin set claim verdict disposition"
    );

    let knob = knob_for(&verdict, &context.evidence_registry);
    Ok((
        StatusCode::OK,
        Json(VerdictDetailResponse {
            verdict: verdict.into(),
            knob,
        }),
    ))
}

/// Handle `GET /api/admin/claim-verdicts/health`.
///
/// Flagged verdicts and their dispositions over the window, broken down by
/// layer, category, agent, reason and day.
///
/// # Errors
///
/// `AppError` on auth failure, an invalid tenant, or a repository error.
pub async fn handle_verdict_health(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<VerdictHealthQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&params.tenant_id)?;

    let tenant = parse_tenant(&params.tenant_id)?;
    let window_days = params.window_days.unwrap_or(30).clamp(1, 365);

    let stats = context
        .repos
        .claim_verdicts
        .aggregate_verdict_health(tenant, window_days)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to aggregate verdict health");
            AppError::internal(format!("Failed to aggregate verdict health: {e}"))
        })?;

    Ok((StatusCode::OK, Json(stats)))
}

/// The knob behind a verdict: where the input its layer read lives.
///
/// Every path here exists in this repository or in dravr-contremaitre; the
/// identifiers named in `detail` are the ones that decide the layer's
/// behaviour. The evidence layer's knob is computed against the registry as
/// it is now, which is the corpus the next verdict will read.
#[must_use]
pub fn knob_for(verdict: &ClaimVerdict, registry: &EvidenceRegistry) -> VerdictKnob {
    let layer = verdict.layer_fired.as_str().to_owned();
    match verdict.layer_fired {
        VerdictLayer::Rhetoric => VerdictKnob {
            layer,
            kind: "rhetoric_filter".to_owned(),
            location: "crates/pierre-evals/src/rhetoric_detector.rs".to_owned(),
            detail: "`classify` marks a claim rhetorical when it opens with one of \
                     RHETORICAL_PREFIXES or carries one of QUESTION_MARKERS. A factual claim \
                     filed as rhetorical needs its opening removed from the prefix list; a \
                     flourish that reached the later layers needs it added."
                .to_owned(),
            propositions: Vec::new(),
        },
        VerdictLayer::Deterministic => VerdictKnob {
            layer,
            kind: "deterministic_bounds".to_owned(),
            location: "crates/pierre-evals/src/deterministic_bounds.rs".to_owned(),
            detail: deterministic_detail(verdict),
            propositions: Vec::new(),
        },
        VerdictLayer::Personalized => VerdictKnob {
            layer,
            kind: "personalized_tolerance".to_owned(),
            location: "crates/pierre-evals/src/personalized.rs".to_owned(),
            detail: "The contradiction margin is DEFAULT_MARGIN_FRAC in personalized.rs, \
                     overridden per agent by `verification_config.personalized.margin_frac` \
                     in the agent's system-prompt frontmatter (PersonalizedConfig in \
                     crates/pierre-evals/src/verification_config.rs); the layer does not fire \
                     under MIN_DATA_DAYS of history. The explanation quotes the athlete's \
                     range the claim fell outside of."
                .to_owned(),
            propositions: Vec::new(),
        },
        VerdictLayer::AthleteData => VerdictKnob {
            layer,
            kind: "athlete_data_record".to_owned(),
            location: "crates/pierre-evals/src/athlete_data.rs".to_owned(),
            detail: "`check` compares the figures the claim asserts against the athlete's \
                     AthleteRecord within MATCH_TOLERANCE; a named activity is matched by \
                     `check_named_activity`. A false positive here is either a record that \
                     was stale when the reply was verified or a tolerance tighter than the \
                     provider's own rounding."
                .to_owned(),
            propositions: Vec::new(),
        },
        VerdictLayer::Evidence => evidence_knob(verdict, registry),
        VerdictLayer::Consistency => VerdictKnob {
            layer,
            kind: "consistency_check".to_owned(),
            location: "crates/pierre-evals/src/consistency.rs".to_owned(),
            detail: "`find_contradiction` pairs this claim with a sibling in the same reply \
                     that shares a subject and disagrees: a negation from NEGATION_MARKERS, \
                     or numbers further apart than NUMERIC_TOLERANCE. The explanation names \
                     the sibling."
                .to_owned(),
            propositions: Vec::new(),
        },
        VerdictLayer::Judge => VerdictKnob {
            layer,
            kind: "judge_prompt".to_owned(),
            location: "prompts/system/claim_judge.md".to_owned(),
            detail:
                "`judge_claim` asked the configured LLM under the `claim_judge` system prompt, \
                     with the retrieved propositions as its evidence block; the explanation \
                     is its rationale verbatim. The judge only runs once the pure-Rust \
                     layers were inconclusive, so a wrong call here is either the prompt or \
                     evidence the corpus lacked."
                    .to_owned(),
            propositions: Vec::new(),
        },
    }
}

/// The bound function the deterministic layer ran for the claim's category.
fn deterministic_detail(verdict: &ClaimVerdict) -> String {
    let probe = match verdict.category {
        ClaimCategory::Physiological => Some("check_physiological"),
        ClaimCategory::TrainingPrescription => Some("check_training"),
        ClaimCategory::Nutrition => Some("check_nutrition"),
        ClaimCategory::Recovery => Some("check_recovery"),
        ClaimCategory::Supplement => Some("check_supplement"),
        ClaimCategory::InjuryRehab | ClaimCategory::AthleteData => None,
    };
    let category = verdict.category.as_str();
    probe.map_or_else(
        || {
            format!(
                "`check` returns no bound for `{category}` by design, so this layer cannot \
                 have produced a verdict for it; the row's category and layer disagree."
            )
        },
        |name| {
            format!(
                "`{name}` holds the population bounds for `{category}`; the explanation names \
                 the bound the claim's figure violated. A false positive is a bound tighter \
                 than what a competent agent can legitimately say, or a number attributed \
                 to the wrong keyword within WINDOW_BYTES of it."
            )
        },
    )
}

/// The evidence knob: the propositions of the claim's category that
/// keyword-match it on the registry as it is now, with the cited ones marked.
fn evidence_knob(verdict: &ClaimVerdict, registry: &EvidenceRegistry) -> VerdictKnob {
    let layer = verdict.layer_fired.as_str().to_owned();
    let category = verdict.category.as_str();
    let cited: Vec<&str> = verdict
        .evidence_refs
        .as_deref()
        .map(|refs| {
            refs.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // An athlete-data claim has no corpus folder by design: it reached the
    // evidence layer only because the athlete-data layer did not fire, and
    // the input to look at is that layer's record, not a proposition.
    if verdict.category == ClaimCategory::AthleteData {
        return VerdictKnob {
            layer,
            kind: "athlete_data_record".to_owned(),
            location: "crates/pierre-evals/src/verdict_engine.rs".to_owned(),
            detail: "The corpus holds no `athlete_data` propositions by design, so this claim \
                     reached the evidence layer because `run_layers_1_to_4` was given no \
                     AthleteRecord: build_athlete_record in \
                     crates/pierre-chat-pipeline/src/stages/verification.rs returned none for \
                     this athlete at verification time. The verdict says nothing about the \
                     claim; the knob is why the record was absent."
                .to_owned(),
            propositions: Vec::new(),
        };
    }

    if registry.is_empty() {
        return VerdictKnob {
            layer,
            kind: "evidence_corpus".to_owned(),
            location: format!("crates/pierre-evals/fixtures/sports_science/{category}/"),
            detail: format!(
                "The evidence registry holds no synced proposition, so retrieval reads the \
                 compiled-in corpus tabled in crates/pierre-services/src/claim_verification.rs \
                 (EMBEDDED_PROPOSITIONS). Cited records: {}. The category's minimum strength \
                 is `verification_config.categories.{category}.min_strength` \
                 (CategoryConfig in crates/pierre-evals/src/verification_config.rs).",
                cited_list(&cited)
            ),
            propositions: Vec::new(),
        };
    }

    let mut propositions: Vec<KnobProposition> = registry
        .list()
        .into_iter()
        .filter(|(c, _, _)| c == category)
        .filter_map(|(c, slug, entry)| {
            let best = entry
                .corpus
                .retrieve(&verdict.claim_text, verdict.category, 1)
                .into_iter()
                .next()?;
            Some(KnobProposition {
                cited: cited.contains(&best.record.id.as_str()),
                path: format!("evidence/sports_science/{c}/{slug}.md"),
                id: best.record.id,
                category: c,
                slug,
                strength: best.record.strength.as_str().to_owned(),
                score: best.score,
            })
        })
        .collect();
    propositions.sort_by(|a, b| {
        b.cited
            .cmp(&a.cited)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    propositions.truncate(KNOB_PROPOSITION_LIMIT);

    let detail = if propositions.is_empty() {
        format!(
            "No proposition under evidence/sports_science/{category}/ shares a 4+ letter \
             word with the claim, which is what `EvidenceCorpus::retrieve` scores on \
             (crates/pierre-evals/src/evidence_retriever.rs). Cited records: {}. A true \
             claim with no match needs a proposition added there, or a keyword added to \
             the one that covers it.",
            cited_list(&cited)
        )
    } else {
        format!(
            "{} proposition(s) under evidence/sports_science/{category}/ keyword-match the \
             claim (crates/pierre-evals/src/evidence_retriever.rs, `retrieve`); the cited \
             ones are marked. A verdict below the required strength is decided by \
             `verification_config.categories.{category}.min_strength` (CategoryConfig in \
             crates/pierre-evals/src/verification_config.rs) against the best match's \
             `strength:` frontmatter.",
            propositions.len()
        )
    };

    VerdictKnob {
        layer,
        kind: "evidence_corpus".to_owned(),
        location: format!("evidence/sports_science/{category}/"),
        detail,
        propositions,
    }
}

fn cited_list(cited: &[&str]) -> String {
    if cited.is_empty() {
        "none".to_owned()
    } else {
        cited.join(", ")
    }
}
