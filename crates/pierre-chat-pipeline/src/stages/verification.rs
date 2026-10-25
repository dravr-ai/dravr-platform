// ABOUTME: Claim verification stage — runs the bullshit detector pipeline over the assistant reply
// ABOUTME: Provides apply_claim_verification — emits ClaimVerdict rows for evidence-strength chips
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Claim verification.
//!
//! Runs the `pierre_evals` heuristic pipeline over the assistant reply to
//! detect unsupported claims, persists verdicts for the admin dashboard,
//! and reacts per the agent's `VerificationConfig` (parsed from YAML
//! frontmatter on the agent's system prompt). Pure-Rust in Phase A, no
//! measurable latency.
//!
//! Gated behind `#[cfg(feature = "tools-verification")]` because the
//! evidence corpus and `pierre_evals` dependency are not built in every
//! workspace configuration.

use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt;
use pierre_core::error_helpers::panic_payload_str;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::athlete_data::AthleteRecord;
use pierre_evals::{
    verify_reply, AthleteMetrics, ClaimJudge, ClaimSource, ExtractedClaim, PersonalizedContext,
    ResolvedAction, ToleranceStrategy, VerdictOutcome, VerificationConfig, VerificationFallback,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, VerdictLayer};

use super::reply_locale::resolve_banner_locale;
use crate::envelope::VerdictChip;
use crate::ChatPipelineContext;
use pierre_contremaitre::messaging_strings::{
    KEY_VERIFICATION_BLOCK_FALLBACK, KEY_VERIFICATION_WARN_SUFFIX,
};
use pierre_core::models::TenantId;
use pierre_llm::call_record::LlmCallRecorder;
use pierre_llm::stage::LlmStage;
use pierre_llm::{ChatProvider, LlmProvider};
use pierre_providers::ai_scope;
use pierre_runtime_context::DataContext;
use pierre_services::athlete_claim_context::{
    build_athlete_record as shared_athlete_record,
    build_personalized_inputs as shared_personalized_inputs, AthleteSources,
};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use uuid::Uuid;

/// Resolve the Layer-5 claim-judge provider, or `None` when the runtime
/// judge is disabled in the harness config or no provider is wired.
///
/// Goes through [`chat_provider_from_resources_arc`], the same factory
/// Stage 11 dispatch and the identity-leak re-ask use. Pub so integration
/// tests can pin the production resolution seam — the previous direct
/// `ctx.llm_provider` read was dead code on every live turn while tests
/// stayed green, because they injected through the seam production leaves
/// empty.
///
/// A resolution failure is a wiring bug, so it warns; the stage then stays
/// deterministic-only rather than failing the turn.
#[must_use]
pub fn resolve_claim_judge(
    runtime_judge: bool,
    chat_provider: Option<&Arc<ChatProvider>>,
    llm_provider: Option<&Arc<dyn LlmProvider>>,
) -> Option<Arc<ChatProvider>> {
    if !runtime_judge {
        return None;
    }
    match chat_provider_from_resources_arc(chat_provider, llm_provider) {
        Ok(provider) => Some(provider),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "runtime claim judge found no provider; verification stays deterministic-only"
            );
            None
        }
    }
}

/// Approximate "lead" of an agent reply for verification-banner deduplication.
///
/// Returns the first ~600 bytes of `reply`, snapped to a UTF-8 char
/// boundary. The Warn-banner builder uses this to decide whether a flagged
/// claim that did not come from the reply itself already appears in the
/// message's opening — listing such text under the banner reads as
/// duplication. Claims extracted from the reply are never matched against it
/// (see [`warning_bullets`]). 600 is wide
/// enough to cover a short safety preamble (e.g. medical disclaimer)
/// followed by the actual lead sentence, but narrow enough that a
/// mid-body claim still escapes the filter and gets listed.
///
/// # Examples
///
/// Short reply is returned untouched:
///
/// ```
/// use pierre_chat_pipeline::stages::verification::lead_window;
/// let reply = "A short reply.";
/// assert_eq!(lead_window(reply), reply);
/// ```
///
/// Long reply is truncated at the lead window:
///
/// ```
/// use pierre_chat_pipeline::stages::verification::lead_window;
/// let reply = "x".repeat(2000);
/// let lead = lead_window(&reply);
/// assert_eq!(lead.len(), 600);
/// ```
///
/// Truncation snaps to a UTF-8 char boundary (no panic mid-codepoint):
///
/// ```
/// use pierre_chat_pipeline::stages::verification::lead_window;
/// let prefix = "x".repeat(599);
/// let reply = format!("{prefix}é trailing tail");
/// let lead = lead_window(&reply);
/// assert!(lead.len() <= 600);
/// assert!(lead.is_char_boundary(lead.len()));
/// ```
#[must_use]
pub fn lead_window(reply: &str) -> &str {
    const LEAD_BYTES: usize = 600;
    if reply.len() <= LEAD_BYTES {
        return reply;
    }
    let mut end = LEAD_BYTES;
    while end > 0 && !reply.is_char_boundary(end) {
        end -= 1;
    }
    &reply[..end]
}

/// Maximum number of flagged claims listed in the user-facing caveat banner.
///
/// The header reads "a few claims…", so a representative subset is consistent
/// with the wording; the full set still lands in the per-message verdict
/// drawer. The cap stops the banner from dwarfing the reply itself.
const MAX_FLAGGED_BULLETS: usize = 5;

/// One flagged claim headed for the user-facing warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlaggedClaim<'a> {
    /// The claim text as extracted.
    pub text: &'a str,
    /// `true` for a bound violation, `false` for a merely unsupported claim.
    pub contradicted: bool,
    /// Where the claim text came from.
    pub source: ClaimSource,
}

impl<'a> FlaggedClaim<'a> {
    fn new(claim: &'a ExtractedClaim, contradicted: bool) -> Self {
        Self {
            text: claim.text.as_str(),
            contradicted,
            source: claim.source,
        }
    }
}

/// Select the actionable problems worth surfacing, each tagged
/// `contradicted` (`true`) vs merely `unsupported` (`false`).
///
/// A [`ClaimCategory::TrainingPrescription`] is advice, not a proposition the
/// evidence corpus can support — a bare `Unsupported` ("no citation") is a
/// category error there, not something to nag the user about. A `Contradicted`
/// prescription DID violate a deterministic bound (e.g. an impossible training
/// load), so it stays surfaced. Every other category keeps both statuses.
#[must_use]
pub fn actionable_problems(verdicts: &[(ExtractedClaim, VerdictOutcome)]) -> Vec<FlaggedClaim<'_>> {
    verdicts
        .iter()
        .filter_map(|(claim, outcome)| {
            actionable_flag(claim, outcome)
                .map(|contradicted| FlaggedClaim::new(claim, contradicted))
        })
        .collect()
}

/// The single source of truth for "is this verdict worth acting on".
///
/// Returns `Some(contradicted)` when it is. A bare `Unsupported` on a
/// [`ClaimCategory::TrainingPrescription`] is a category error (advice has no
/// citation to support), so it is not actionable; a `Contradicted` prescription
/// violated a bound and stays.
fn actionable_flag(claim: &ExtractedClaim, outcome: &VerdictOutcome) -> Option<bool> {
    match outcome.status {
        ClaimStatus::Contradicted => Some(true),
        ClaimStatus::Unsupported if claim.category != ClaimCategory::TrainingPrescription => {
            Some(false)
        }
        _ => None,
    }
}

/// The dispatch action for a single verdict.
///
/// Personalized verdicts route through the agent's
/// [`ContradictionPolicy`]; every other layer keeps the existing
/// `fallback_behavior` mapping, so non-personalized behavior is unchanged.
fn resolved_action(outcome: &VerdictOutcome, config: &VerificationConfig) -> ResolvedAction {
    if outcome.is_judge_error() {
        // A claim the judge never checked has no verdict to warn about, so
        // only an agent that withholds unchecked replies acts on it; every
        // other agent ships the reply and keeps the "not checked" row.
        return if withholds_unchecked_reply(config) {
            ResolvedAction::BlockRetry
        } else {
            ResolvedAction::RecordOnly
        };
    }
    if outcome.layer_fired == VerdictLayer::Personalized {
        config
            .personalized
            .contradiction_policy()
            .resolve(outcome, config)
    } else {
        match config.fallback_behavior {
            VerificationFallback::Warn => ResolvedAction::WarnBanner,
            VerificationFallback::Silent => ResolvedAction::RecordOnly,
            VerificationFallback::Block => ResolvedAction::BlockRetry,
        }
    }
}

/// The strongest action across the reply's actionable verdicts and its
/// unchecked claims. Every other verdict never drives a reply change, matching
/// [`actionable_problems`].
fn reply_action(
    verdicts: &[(ExtractedClaim, VerdictOutcome)],
    config: &VerificationConfig,
) -> ResolvedAction {
    verdicts
        .iter()
        .filter(|(claim, outcome)| {
            actionable_flag(claim, outcome).is_some() || outcome.is_judge_error()
        })
        .map(|(_, outcome)| resolved_action(outcome, config))
        .max_by_key(|action| action_rank(*action))
        .unwrap_or(ResolvedAction::Pass)
}

/// Severity ordering so [`reply_action`] can take the strongest action.
const fn action_rank(action: ResolvedAction) -> u8 {
    match action {
        ResolvedAction::Pass => 0,
        ResolvedAction::RecordOnly => 1,
        ResolvedAction::WarnBanner => 2,
        ResolvedAction::BlockRetry => 3,
    }
}

/// The single user-facing affordance for a turn's flagged claims.
///
/// One flagged claim earns one warning. Which shape it takes is a surface
/// capability — a chip rail, or a caveat written into the reply — and the two
/// are variants of one enum rather than two independent outputs, so a surface
/// cannot receive both. Web shipped banner *and* chips for a single claim
/// before this type existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarnAffordance {
    /// Structured chips beside an untouched reply.
    Chips(Vec<VerdictChip>),
    /// A caveat appended to the reply text, for a surface with no chip rail.
    Banner(String),
    /// Nothing worth surfacing — no claim resolved to a user-facing warning.
    Silent,
}

/// Build the one affordance a set of flagged claims earns.
///
/// `renders_chips` is the surface capability, not a channel name.
#[must_use]
pub fn warn_affordance(
    shown: &[FlaggedClaim<'_>],
    reply: &str,
    renders_chips: bool,
    banner_header: &str,
) -> WarnAffordance {
    if renders_chips {
        if shown.is_empty() {
            return WarnAffordance::Silent;
        }
        return WarnAffordance::Chips(
            shown
                .iter()
                .map(|flagged| VerdictChip {
                    claim: flagged.text.trim().to_owned(),
                    contradicted: flagged.contradicted,
                })
                .collect(),
        );
    }
    let bullets = warning_bullets(shown, reply);
    if bullets.is_empty() {
        return WarnAffordance::Silent;
    }
    let body = bullets.join("\n");
    WarnAffordance::Banner(format!("{reply}\n\n---\n{banner_header}\n{body}"))
}

/// Format the caveat bullets from the actionable problems.
///
/// Lists each flagged claim verbatim so the reader can challenge or wave
/// through the specific sentence (an opaque "I'm unsure about N things"
/// trailer just undermined credibility without being actionable — Telegram
/// nutrition-recommendation incident, 2026-05-08).
///
/// A [`ClaimSource::Reply`] claim is always kept: it is by construction a
/// sentence of `reply`, so finding it in the lead window only says where the
/// flagged sentence sits, not that the warning is redundant. Dropping those
/// silenced the caveat on every short reply. Only text from another source
/// that already appears in the lead window is dropped as an echo. Bound
/// violations (`contradicted`) sort ahead of merely-unsupported claims and
/// survive the [`MAX_FLAGGED_BULLETS`] cap.
#[must_use]
pub fn warning_bullets(problems: &[FlaggedClaim<'_>], reply: &str) -> Vec<String> {
    let lead = lead_window(reply);
    let mut flagged: Vec<FlaggedClaim<'_>> = problems
        .iter()
        .copied()
        .filter(|claim| claim.source == ClaimSource::Reply || !lead.contains(claim.text.trim()))
        .collect();
    flagged.sort_by_key(|claim| u8::from(!claim.contradicted));
    let total_flagged = flagged.len();
    flagged.truncate(MAX_FLAGGED_BULLETS);
    if total_flagged > flagged.len() {
        tracing::info!(
            shown = flagged.len(),
            total = total_flagged,
            "claim verification truncated the flagged-claim warning list"
        );
    }
    flagged
        .iter()
        .map(|claim| format!("- {}", claim.text.trim()))
        .collect()
}

/// Inputs to [`apply_claim_verification`].
///
/// Bundles the claim-verification turn context so the function signature stays under
/// clippy's `too_many_arguments` ceiling. All fields are borrowed from
/// caller-owned state — no ownership transfer, no cloning.
pub struct ClaimVerificationParams<'a> {
    /// Pipeline context providing the registries and corpus.
    pub ctx: &'a ChatPipelineContext,
    /// `true` when the surface attaches verdicts to the reply as chips.
    ///
    /// The one input that decides which of the two affordances a flagged claim
    /// gets. When set, the Warn path returns [`ClaimVerificationOutcome::chips`]
    /// and leaves the reply text alone; when clear, it appends the caveat
    /// banner and returns no chips. Never both — the web surface used to draw
    /// the banner *and* the chips for a single flagged claim, telling the
    /// athlete the same thing twice in two registers.
    pub renders_chips: bool,
    /// Assistant reply text to scan.
    pub reply: &'a str,
    /// Parsed verification config (from the agent's prompt frontmatter).
    pub config: &'a VerificationConfig,
    /// The turn's resolved locale, used for the Warn/Block fallback strings
    /// when the reply's own language cannot be detected.
    pub locale: &'a str,
    /// User whose physiology backs the personalized snapshot.
    pub user_id: &'a str,
    /// Tenant owning the user's data (multi-tenant scoping for snapshot reads).
    pub tenant_id: TenantId,
    /// Receives one usage record per claim the LLM judge is asked about,
    /// typed [`LlmStage::ClaimJudge`].
    pub judge_recorder: Option<&'a Arc<dyn LlmCallRecorder>>,
}

impl fmt::Debug for ClaimVerificationParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClaimVerificationParams")
            .field("ctx", &self.ctx)
            .field("renders_chips", &self.renders_chips)
            .field("reply_len", &self.reply.len())
            .field("config", &self.config)
            .field("locale", &self.locale)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}

/// Result of running claim verification on an assistant reply.
///
/// Callers must persist `pending_verdicts` after the assistant message is
/// durable; each verdict carries the claim and outcome fields needed to build
/// an [`InsertClaimVerdictParams`] linked to the stored message. Emitting the
/// verdict rows before the message exists would leave them with a `None`
/// `message_id` and orphan them if the message write failed.
pub struct ClaimVerificationOutcome {
    /// Finalized reply text after fallback handling.
    pub content: String,
    /// Raw verdicts produced by the detector, kept together with their claims
    /// so the caller can persist them with the assistant `message_id`.
    pub pending_verdicts: Vec<(ExtractedClaim, VerdictOutcome)>,
    /// The flagged claims to attach to the reply, populated only when
    /// [`ClaimVerificationParams::renders_chips`] is set — which is exactly
    /// when [`Self::content`] was left without a caveat banner.
    pub chips: Vec<VerdictChip>,
}

// The reply and every claim quoted from it (the pending verdicts and the
// chips) are model text: only the reply's size and the counts are printed.
impl fmt::Debug for ClaimVerificationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClaimVerificationOutcome")
            .field("content_len", &self.content.len())
            .field("pending_verdicts_len", &self.pending_verdicts.len())
            .field("chips_len", &self.chips.len())
            .finish()
    }
}

/// Whether a reply the stage could not fully check is withheld.
///
/// The one fallback policy for every way verification can come up short: a
/// panic in the detector ([`degrade_to_unverified`]) and a judge call that
/// failed on a claim ([`resolved_action`]). An agent whose
/// [`VerificationFallback::Block`] exists to REPLACE a reply carrying a
/// contradicted claim must not deliver one nobody checked, and every other
/// agent delivers it. A disabled config never withholds anything: the stage
/// returns the reply untouched before it looks at a single claim.
fn withholds_unchecked_reply(config: &VerificationConfig) -> bool {
    config.enabled && config.fallback_behavior == VerificationFallback::Block
}

/// Run `stage` under a panic boundary, degrading per the agent's configured
/// [`VerificationConfig::fallback_behavior`].
///
/// Claim verification is decorative and it runs *late*: by the time it sees
/// the reply, the turn's tool calls have already committed their writes. A
/// panic here must therefore cost the reply's footer, never the turn.
///
/// On 2026-07-28 it cost the turn. A byte-offset window in the
/// deterministic-bounds scanner sliced through an accented character in a
/// French agent reply; the panic unwound past post-processing to the
/// turn-level boundary in messaging dispatch, which correctly reported a
/// total failure — six seconds after `save_training_plan` had committed the
/// athlete's first successful plan. He was shown an outage message for a plan
/// that was in the database.
///
/// The boundary is deliberately this narrow rather than wrapped around all of
/// post-processing. Neighbouring stages are not decorative: the identity-leak
/// scrub is a response-boundary security control, and degrading *it* to the
/// raw model output on panic would publish the very text it exists to
/// withhold. Only stages whose output can be dropped without changing what
/// the turn is allowed to say belong inside a boundary like this one.
///
/// Delivering the unverified reply is right for an agent that would only have
/// appended a banner or recorded the verdict. It is wrong for one whose
/// [`VerificationFallback::Block`] exists to REPLACE a reply carrying a
/// contradicted claim — "an HR max of 300 bpm", "500 g of creatine per day" are
/// the class the deterministic bounds catch, and they are precisely the class
/// that panics the scanner. An unscanned reply is exactly as unproven as a
/// flagged one, so a blocking agent gets its block fallback, which the caller
/// supplies through `block_fallback` (localized, so it needs the messaging
/// registry). The closure is only called when the agent blocks *and* the stage
/// panicked.
///
/// `AssertUnwindSafe` is sound because a caught panic discards the stage's
/// result whole: nothing of the stage's own state is read afterwards and no
/// verdicts are persisted, so no partially-updated state is ever observed.
pub async fn degrade_to_unverified<F, B>(
    stage: F,
    reply: &str,
    config: &VerificationConfig,
    block_fallback: B,
) -> ClaimVerificationOutcome
where
    F: Future<Output = ClaimVerificationOutcome>,
    B: FnOnce() -> String,
{
    match AssertUnwindSafe(stage).catch_unwind().await {
        Ok(outcome) => outcome,
        Err(payload) => {
            let blocks = withholds_unchecked_reply(config);
            tracing::error!(
                panic = %panic_payload_str(payload.as_ref()),
                blocked = blocks,
                "claim verification panicked — degrading per the coach's fallback behavior rather than discarding the turn"
            );
            ClaimVerificationOutcome {
                content: if blocks {
                    block_fallback()
                } else {
                    reply.to_owned()
                },
                pending_verdicts: Vec::new(),
                chips: Vec::new(),
            }
        }
    }
}

/// Run the bullshit detector over the finalized assistant reply.
///
/// Computes verdicts and applies the agent's
/// [`VerificationConfig::fallback_behavior`] to the reply, but defers
/// persisting verdicts. The caller is expected to write the assistant message
/// first and then invoke [`persist_pending_verdicts`] with the resulting
/// `message_id`. This keeps the audit row linked to the message that
/// produced it and avoids orphan verdicts when the message write fails.
///
/// Wrapped in [`degrade_to_unverified`] so that a bug anywhere in the
/// detector costs the verification footer and nothing else.
pub async fn apply_claim_verification(
    params: ClaimVerificationParams<'_>,
) -> ClaimVerificationOutcome {
    // Copied out before `params` moves; every one is a shared reference, which
    // is Copy and outlives the call.
    let reply = params.reply;
    let config = params.config;
    let ctx = params.ctx;
    let locale = params.locale;
    // Rendering the block fallback costs a language detection, so it is built
    // lazily: only a blocking agent whose stage actually panicked pays for it.
    degrade_to_unverified(verify_and_apply(params), reply, config, || {
        ctx.messaging_strings_registry.get(
            KEY_VERIFICATION_BLOCK_FALLBACK,
            &resolve_banner_locale(reply, locale),
        )
    })
    .await
}

/// The claim-verification stage proper. Always entered through
/// [`apply_claim_verification`], which owns the panic boundary.
async fn verify_and_apply(params: ClaimVerificationParams<'_>) -> ClaimVerificationOutcome {
    let ClaimVerificationParams {
        ctx,
        renders_chips,
        reply,
        config,
        locale,
        user_id,
        tenant_id,
        judge_recorder,
    } = params;
    let locale = resolve_banner_locale(reply, locale);
    let locale = locale.as_str();

    if !config.enabled {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
            chips: Vec::new(),
        };
    }

    let corpus = ctx.evidence_registry.resolved_corpus();
    // The LLM-judge layer (Layer 5), resolved through the same provider
    // factory Stage 11 dispatch and the identity-leak re-ask use — reading
    // `ctx.llm_provider` directly made the judge dead code in production,
    // where the binary wires `chat_provider` and leaves `llm_provider: None`
    // (the exact trap documented on `resolve_reask_provider`). Gated by the
    // hot-reloadable harness config so operators can drop back to
    // deterministic-only at runtime.
    let judge_provider = resolve_claim_judge(
        ctx.harness_config_registry
            .current_verification()
            .runtime_judge,
        ctx.chat_provider.as_ref(),
        ctx.llm_provider.as_ref(),
    );
    // The judge's instructions are the catalogue's `claim_judge` prompt, read
    // per turn so an edit there reaches the next verification without a deploy.
    let judge_prompt = ctx.prompt_registry.claim_judge_prompt();
    // The judge runs on its stage model when the provider is the platform's
    // head, and every claim it is asked about writes its own usage row.
    let judge: Option<ClaimJudge<'_>> = judge_provider.as_deref().map(|p| ClaimJudge {
        provider: p,
        system_prompt: judge_prompt.trim(),
        model: p.stage_model(LlmStage::ClaimJudge),
        recorder: judge_recorder,
    });

    // The personalized layer — build the athlete snapshot + tolerance strategy when the agent
    // enabled personalized verification. The snapshot owns its data so its
    // borrow lives through the verify call; an unusable snapshot (thin history)
    // makes the layer a silent no-op. Kept in fn-scope so `personalized` can
    // borrow it.
    let personalized_inputs: Option<(AthleteMetrics, Box<dyn ToleranceStrategy>)> =
        build_personalized_inputs(ctx, config, user_id, tenant_id).await;
    let personalized =
        personalized_inputs
            .as_ref()
            .map(|(metrics, tolerance)| PersonalizedContext {
                metrics,
                tolerance: tolerance.as_ref(),
            });

    // The athlete-data layer's inputs. Unlike the personalized layer this is
    // built unconditionally: its most important verdict is the one it reaches
    // when the athlete has *nothing* connected, and a snapshot that opted out
    // of being built could never deliver it.
    let athlete_record = build_athlete_record(ctx, user_id, tenant_id, locale).await;

    let verdicts = verify_reply(
        reply,
        config,
        &corpus,
        judge,
        personalized.as_ref(),
        athlete_record.as_ref(),
    )
    .await;
    if verdicts.is_empty() {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
            chips: Vec::new(),
        };
    }

    let problems = actionable_problems(&verdicts);
    let unchecked_claims = verdicts
        .iter()
        .filter(|(_, outcome)| outcome.is_judge_error())
        .count();
    if unchecked_claims > 0 {
        // The line the `dravr-claim-verification-failed` metric counts
        // (infra/environments/dev/claim_verification_monitoring.tf): a reply
        // went out with claims nobody checked.
        tracing::warn!(
            unchecked_claims,
            blocked = withholds_unchecked_reply(config),
            "claim verification failed — judge error, claims recorded as not checked"
        );
    }

    let mut chips = Vec::new();
    let content = if problems.is_empty() && unchecked_claims == 0 {
        reply.to_owned()
    } else {
        match reply_action(&verdicts, config) {
            ResolvedAction::WarnBanner => {
                // Only list claims whose own resolved action is a user-facing
                // warning — an audit-only personalized contradiction is recorded
                // but must never surface in the banner.
                let shown: Vec<FlaggedClaim<'_>> = verdicts
                    .iter()
                    .filter_map(|(claim, outcome)| {
                        actionable_flag(claim, outcome).and_then(|contradicted| {
                            (resolved_action(outcome, config) == ResolvedAction::WarnBanner)
                                .then(|| FlaggedClaim::new(claim, contradicted))
                        })
                    })
                    .collect();
                // One affordance per flagged claim. A surface that attaches
                // chips gets them and an untouched reply; a surface without
                // chips gets the caveat banner written into the reply. The
                // branch is here, at the single place the warning is produced,
                // so the two cannot both fire.
                let header = ctx
                    .messaging_strings_registry
                    .get(KEY_VERIFICATION_WARN_SUFFIX, locale);
                match warn_affordance(&shown, reply, renders_chips, &header) {
                    WarnAffordance::Chips(built) => {
                        chips = built;
                        reply.to_owned()
                    }
                    WarnAffordance::Banner(text) => text,
                    WarnAffordance::Silent => reply.to_owned(),
                }
            }
            ResolvedAction::Pass | ResolvedAction::RecordOnly => reply.to_owned(),
            ResolvedAction::BlockRetry => {
                tracing::warn!(
                    flagged_claims = problems.len(),
                    unchecked_claims,
                    "claim-verification block fallback fired — replacing reply"
                );
                ctx.messaging_strings_registry
                    .get(KEY_VERIFICATION_BLOCK_FALLBACK, locale)
            }
        }
    };

    ClaimVerificationOutcome {
        content,
        pending_verdicts: verdicts,
        chips,
    }
}

/// The sources the turn's athlete context is read from, or `None` when the
/// user id does not parse.
///
/// Activities are filtered by the tool runtime's provider registry, the terms
/// the turn's own tool calls were served under: a claim is checked against
/// what the model could have seen (carnet#723).
fn athlete_sources<'a>(
    ctx: &'a ChatPipelineContext,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<AthleteSources<'a>> {
    Some(AthleteSources {
        repos: ctx.repos.as_ref(),
        policies: ctx.tool_runtime.provider_registry().as_ref(),
        tenant_id,
        user_id: Uuid::parse_str(user_id).ok()?,
    })
}

/// Gather what we hold about this athlete's own records, through the builder
/// every verifying surface shares
/// ([`pierre_services::athlete_claim_context::build_athlete_record`]).
///
/// Returns `None` when the user id will not parse or the athlete's connection
/// state cannot be established — skipping the athlete-data layer rather than
/// risking a contradiction of a true claim.
async fn build_athlete_record(
    ctx: &ChatPipelineContext,
    user_id: &str,
    tenant_id: TenantId,
    locale: &str,
) -> Option<AthleteRecord> {
    let sources = athlete_sources(ctx, user_id, tenant_id)?;
    shared_athlete_record(&sources, Some(locale)).await
}

/// Build the personalized-layer inputs (athlete snapshot + tolerance strategy) for a turn.
///
/// Returns `None` when personalized verification is disabled, the user id is
/// unparseable, or the snapshot is too thin to trust
/// ([`AthleteMetrics::is_usable`]).
async fn build_personalized_inputs(
    ctx: &ChatPipelineContext,
    config: &VerificationConfig,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<(AthleteMetrics, Box<dyn ToleranceStrategy>)> {
    let sources = athlete_sources(ctx, user_id, tenant_id)?;
    let cageux = ctx.cageux_config_registry.current();
    shared_personalized_inputs(
        &sources,
        &cageux.algorithms,
        &ctx.config.training_zones,
        &config.personalized,
    )
    .await
}

/// Persist the verdicts produced by [`apply_claim_verification`].
///
/// The caller invokes this after the assistant message has been stored so the
/// `message_id` can link each verdict to the reply it came from — the admin
/// UI uses that link to drill into the full verification history behind any
/// flagged message. Writes are best-effort: a single row failing is logged
/// and does not affect the user-facing turn.
pub async fn persist_pending_verdicts(
    data: &DataContext,
    tenant_id: TenantId,
    user_id: &str,
    conversation_id: &str,
    agent_id: Option<&str>,
    message_id: &str,
    pending: &[(ExtractedClaim, VerdictOutcome)],
) {
    for (claim, outcome) in pending {
        let params = InsertClaimVerdictParams {
            tenant_id,
            user_id,
            agent_id,
            conversation_id: Some(conversation_id),
            message_id: Some(message_id),
            claim_text: &claim.text,
            category: claim.category,
            status: outcome.status,
            evidence_strength: outcome.evidence_strength,
            confidence: outcome.confidence,
            layer_fired: outcome.layer_fired,
            explanation: Some(&outcome.explanation),
            evidence_refs: outcome.evidence_refs.as_deref(),
            transport_policy: ai_scope::derived_policy(),
        };
        if let Err(e) = data
            .repos()
            .claim_verdicts
            .insert_claim_verdict(&params)
            .await
        {
            tracing::warn!(error = %e, "failed to persist claim verdict");
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_evals::{
        ClaimSource, ExtractedClaim, ResolvedAction, VerdictOutcome, VerificationConfig,
        VerificationFallback,
    };
    use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

    use super::{reply_action, resolved_action, withholds_unchecked_reply};

    fn config(fallback: VerificationFallback) -> VerificationConfig {
        VerificationConfig {
            fallback_behavior: fallback,
            ..VerificationConfig::default()
        }
    }

    fn claim(text: &str) -> ExtractedClaim {
        ExtractedClaim {
            text: text.to_owned(),
            category: ClaimCategory::Physiological,
            source: ClaimSource::Reply,
        }
    }

    fn verdict(status: ClaimStatus, layer: VerdictLayer) -> VerdictOutcome {
        VerdictOutcome {
            status,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.9,
            layer_fired: layer,
            explanation: String::new(),
            evidence_refs: None,
        }
    }

    fn supported() -> (ExtractedClaim, VerdictOutcome) {
        (
            claim("Sleep 8 hours per night to recover."),
            verdict(ClaimStatus::Supported, VerdictLayer::Evidence),
        )
    }

    fn unsupported() -> (ExtractedClaim, VerdictOutcome) {
        (
            claim("Beetroot juice raises your lactate threshold by 12%."),
            verdict(ClaimStatus::Unsupported, VerdictLayer::Evidence),
        )
    }

    fn unchecked() -> (ExtractedClaim, VerdictOutcome) {
        (
            claim("Tart cherry juice improves your sleep quality by 8%."),
            VerdictOutcome::judge_error(),
        )
    }

    #[test]
    fn an_unchecked_claim_blocks_an_agent_that_blocks() {
        let action = resolved_action(
            &VerdictOutcome::judge_error(),
            &config(VerificationFallback::Block),
        );

        assert_eq!(action, ResolvedAction::BlockRetry);
    }

    #[test]
    fn an_unchecked_claim_is_only_recorded_for_an_agent_that_warns_or_stays_silent() {
        for fallback in [VerificationFallback::Warn, VerificationFallback::Silent] {
            let action = resolved_action(&VerdictOutcome::judge_error(), &config(fallback));

            assert_eq!(action, ResolvedAction::RecordOnly, "{fallback:?}");
        }
    }

    #[test]
    fn a_disabled_config_withholds_nothing() {
        let disabled = VerificationConfig {
            enabled: false,
            ..config(VerificationFallback::Block)
        };

        assert!(!withholds_unchecked_reply(&disabled));
        assert!(withholds_unchecked_reply(&config(
            VerificationFallback::Block
        )));
    }

    #[test]
    fn a_blocking_agent_blocks_a_reply_with_an_unchecked_claim_and_no_flagged_one() {
        let verdicts = vec![supported(), unchecked()];

        let action = reply_action(&verdicts, &config(VerificationFallback::Block));

        assert_eq!(action, ResolvedAction::BlockRetry);
    }

    #[test]
    fn a_blocking_agent_passes_a_reply_whose_claims_were_all_checked() {
        let verdicts = vec![supported()];

        let action = reply_action(&verdicts, &config(VerificationFallback::Block));

        assert_eq!(
            action,
            ResolvedAction::Pass,
            "it is the unchecked claim that blocks, not the agent's setting"
        );
    }

    #[test]
    fn a_warning_agent_ships_a_reply_with_an_unchecked_claim_and_records_it() {
        let verdicts = vec![supported(), unchecked()];

        let action = reply_action(&verdicts, &config(VerificationFallback::Warn));

        assert_eq!(action, ResolvedAction::RecordOnly);
    }

    #[test]
    fn a_flagged_claim_still_earns_its_warning_beside_an_unchecked_one() {
        let verdicts = vec![unchecked(), unsupported()];

        let action = reply_action(&verdicts, &config(VerificationFallback::Warn));

        assert_eq!(action, ResolvedAction::WarnBanner);
    }
}
