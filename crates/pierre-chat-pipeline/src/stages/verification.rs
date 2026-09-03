// ABOUTME: Claim verification stage — runs the bullshit detector pipeline over the assistant reply
// ABOUTME: Provides apply_claim_verification — emits ClaimVerdict rows for evidence-strength chips
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Claim verification.
//!
//! Runs the `pierre_evals` heuristic pipeline over the assistant reply to
//! detect unsupported claims, persists verdicts for the admin dashboard,
//! and reacts per the coach's `VerificationConfig` (parsed from YAML
//! frontmatter on the coach's system prompt). Pure-Rust in Phase A, no
//! measurable latency.
//!
//! Gated behind `#[cfg(feature = "tools-verification")]` because the
//! evidence corpus and `pierre_evals` dependency are not built in every
//! workspace configuration.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt;
use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::error_helpers::panic_payload_str;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::athlete_data::{AthleteRecord, RecordedActivity};
use pierre_evals::{
    AthleteMetrics, ExtractedClaim, PersonalizedContext, ResolvedAction, ToleranceStrategy,
    VerdictOutcome, VerificationConfig, VerificationFallback,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, VerdictLayer};

use crate::envelope::VerdictChip;
use crate::ChatPipelineContext;
use pierre_contremaitre::messaging_strings::{
    KEY_VERIFICATION_BLOCK_FALLBACK, KEY_VERIFICATION_WARN_SUFFIX,
};
use pierre_core::models::TenantId;
use pierre_llm::{ChatProvider, LlmProvider};
use pierre_runtime_context::DataContext;
use pierre_services::athlete_snapshot::build_athlete_metrics;
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_services::claim_verification::resolve_corpus;
use pierre_services::claim_verification::verify_reply_with_config_and_judge;
use pierre_services::onboarding_gate::user_has_connected_provider;
use tracing::warn;
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

/// Localizes the verification warn / block-fallback strings.
///
/// The claim-verification banner is appended verbatim to the LLM's reply, so the
/// banner's language must match the reply's language — otherwise an
/// English session ends with a French postscript (or vice versa).
///
/// Resolution order (returns first match):
/// 1. **Reply text via whatlang** — long replies (≥ a few sentences)
///    detect reliably even for casual conversational tone, which is the
///    case the per-turn locale heuristic in `messaging_ingress` misses
///    (a 4-word user question can't be detected, but the 200-word reply
///    can).
/// 2. **The turn's resolved `locale`** — [`crate::SurfaceProfile::locale`],
///    settled once at the ingress boundary from the user's input, the
///    channel link, and `users.locale`. Honored verbatim whenever the
///    reply's own language is inconclusive.
pub(crate) fn resolve_banner_locale(reply: &str, locale: &str) -> String {
    if let Some(info) = whatlang::detect(reply) {
        if info.is_reliable() {
            let detected = match info.lang() {
                whatlang::Lang::Fra => Some("fr"),
                whatlang::Lang::Eng => Some("en"),
                whatlang::Lang::Spa => Some("es"),
                whatlang::Lang::Deu => Some("de"),
                whatlang::Lang::Por => Some("pt"),
                _ => None,
            };
            if let Some(code) = detected {
                return code.to_owned();
            }
        }
    }
    locale.to_owned()
}

/// Approximate "lead" of a coach reply for verification-banner deduplication.
///
/// Returns the first ~600 bytes of `reply`, snapped to a UTF-8 char
/// boundary. The Warn-banner builder uses this to decide whether a flagged
/// claim is essentially the message's opening sentence — listing such a
/// claim verbatim under the banner reads as duplication. 600 is wide
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

/// Select the actionable problems worth surfacing, each tagged
/// `contradicted` (`true`) vs merely `unsupported` (`false`).
///
/// A [`ClaimCategory::TrainingPrescription`] is advice, not a proposition the
/// evidence corpus can support — a bare `Unsupported` ("no citation") is a
/// category error there, not something to nag the user about. A `Contradicted`
/// prescription DID violate a deterministic bound (e.g. an impossible training
/// load), so it stays surfaced. Every other category keeps both statuses.
#[must_use]
pub fn actionable_problems(verdicts: &[(ExtractedClaim, VerdictOutcome)]) -> Vec<(&str, bool)> {
    verdicts
        .iter()
        .filter_map(|(claim, outcome)| {
            actionable_flag(claim, outcome).map(|contradicted| (claim.text.as_str(), contradicted))
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
/// Personalized verdicts route through the coach's
/// [`ContradictionPolicy`]; every other layer keeps the existing
/// `fallback_behavior` mapping, so non-personalized behavior is unchanged.
fn resolved_action(outcome: &VerdictOutcome, config: &VerificationConfig) -> ResolvedAction {
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

/// The strongest action across the reply's actionable verdicts. Non-actionable
/// verdicts never drive a reply change, matching [`actionable_problems`].
fn reply_action(
    verdicts: &[(ExtractedClaim, VerdictOutcome)],
    config: &VerificationConfig,
) -> ResolvedAction {
    verdicts
        .iter()
        .filter(|(claim, outcome)| actionable_flag(claim, outcome).is_some())
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
    shown: &[(&str, bool)],
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
                .map(|&(claim, contradicted)| VerdictChip {
                    claim: claim.trim().to_owned(),
                    contradicted,
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
/// nutrition-recommendation incident, 2026-05-08). Claims whose text already
/// lands in the reply's lead window are dropped: the reader just saw that
/// sentence as the opening, so echoing it reads as duplication. Bound
/// violations (`contradicted`) sort ahead of merely-unsupported claims and
/// survive the [`MAX_FLAGGED_BULLETS`] cap; if every flagged claim is a
/// lead-window echo the result is empty and the caller suppresses the banner.
#[must_use]
pub fn warning_bullets(problems: &[(&str, bool)], reply: &str) -> Vec<String> {
    let lead = lead_window(reply);
    let mut flagged: Vec<(&str, bool)> = problems
        .iter()
        .copied()
        .filter(|(claim, _)| !lead.contains(claim.trim()))
        .collect();
    flagged.sort_by_key(|&(_, contradicted)| u8::from(!contradicted));
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
        .map(|(claim, _)| format!("- {}", claim.trim()))
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
    /// Parsed verification config (from the coach's prompt frontmatter).
    pub config: &'a VerificationConfig,
    /// The turn's resolved locale, used for the Warn/Block fallback strings
    /// when the reply's own language cannot be detected.
    pub locale: &'a str,
    /// User whose physiology backs the personalized snapshot.
    pub user_id: &'a str,
    /// Tenant owning the user's data (multi-tenant scoping for snapshot reads).
    pub tenant_id: TenantId,
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

/// Run `stage` under a panic boundary, degrading per the coach's configured
/// [`VerificationConfig::fallback_behavior`].
///
/// Claim verification is decorative and it runs *late*: by the time it sees
/// the reply, the turn's tool calls have already committed their writes. A
/// panic here must therefore cost the reply's footer, never the turn.
///
/// On 2026-07-28 it cost the turn. A byte-offset window in the
/// deterministic-bounds scanner sliced through an accented character in a
/// French coach reply; the panic unwound past post-processing to the
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
/// Delivering the unverified reply is right for a coach that would only have
/// appended a banner or recorded the verdict. It is wrong for one whose
/// [`VerificationFallback::Block`] exists to REPLACE a reply carrying a
/// contradicted claim — "an HR max of 300 bpm", "500 g of creatine per day" are
/// the class the deterministic bounds catch, and they are precisely the class
/// that panics the scanner. An unscanned reply is exactly as unproven as a
/// flagged one, so a blocking coach gets its block fallback, which the caller
/// supplies through `block_fallback` (localized, so it needs the messaging
/// registry). The closure is only called when the coach blocks *and* the stage
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
            // A disabled config never blocks anything: the stage returns the
            // reply untouched before it looks at a single claim.
            let blocks = config.enabled && config.fallback_behavior == VerificationFallback::Block;
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
/// Computes verdicts and applies the coach's
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
    // lazily: only a blocking coach whose stage actually panicked pays for it.
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

    let corpus = resolve_corpus(&ctx.evidence_registry);
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
    let judge: Option<&dyn LlmProvider> = judge_provider.as_deref().map(|p| p as &dyn LlmProvider);

    // The personalized layer — build the athlete snapshot + tolerance strategy when the coach
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
    let athlete_record = build_athlete_record(ctx, user_id, tenant_id).await;

    let verdicts = match verify_reply_with_config_and_judge(
        reply,
        config,
        &corpus,
        judge,
        personalized.as_ref(),
        athlete_record.as_ref(),
    )
    .await
    {
        Ok(verdicts) => verdicts,
        Err(e) => {
            tracing::warn!(error = %e, "claim verification failed — skipping claim verdicts");
            return ClaimVerificationOutcome {
                content: reply.to_owned(),
                pending_verdicts: Vec::new(),
                chips: Vec::new(),
            };
        }
    };
    if verdicts.is_empty() {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
            chips: Vec::new(),
        };
    }

    let problems = actionable_problems(&verdicts);

    let mut chips = Vec::new();
    let content = if problems.is_empty() {
        reply.to_owned()
    } else {
        match reply_action(&verdicts, config) {
            ResolvedAction::WarnBanner => {
                // Only list claims whose own resolved action is a user-facing
                // warning — an audit-only personalized contradiction is recorded
                // but must never surface in the banner.
                let shown: Vec<(&str, bool)> = verdicts
                    .iter()
                    .filter_map(|(claim, outcome)| {
                        actionable_flag(claim, outcome).and_then(|contradicted| {
                            (resolved_action(outcome, config) == ResolvedAction::WarnBanner)
                                .then_some((claim.text.as_str(), contradicted))
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

/// Gather what we hold about this athlete's own records.
///
/// Two facts, from two sources that disagree in an important way: whether any
/// provider is connected at all (`provider_connections`, the same table the
/// onboarding gate reads), and what activities we actually hold (the cache).
/// The first is what licenses a contradiction — no provider means a specific
/// figure had no source — while the second only ever supports or fails to
/// support one, because the cache is a window and can legitimately miss a run.
///
/// Returns `None` when the user id will not parse, and — importantly — when the
/// connection lookup *fails*. Both mean we cannot establish the athlete's state,
/// and this is the one layer where guessing is dangerous in a specific
/// direction: "providerless" is what licenses a `Contradicted` verdict at 0.95
/// confidence, saying the coach invented a figure. Collapsing an `Err` into
/// `false` would let a pool timeout brand an accurate reply a fabrication and
/// persist that accusation. Skipping the layer costs a verdict; guessing costs
/// the athlete's trust, so absence of knowledge must never read as knowledge of
/// absence.
///
/// `has_provider` mirrors [`user_has_connected_provider`] plus the `oauth_tokens`
/// second source, exactly as the dispatch chokepoint does, because the two
/// tables are known to drift. An athlete holding a live token whose connection
/// row is missing is served by every tool, so calling them providerless here
/// would contradict every true figure they were just given.
async fn build_athlete_record(
    ctx: &ChatPipelineContext,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<AthleteRecord> {
    let uuid = Uuid::parse_str(user_id).ok()?;

    let connected = match user_has_connected_provider(&ctx.repos.provider_connections, uuid).await {
        Ok(true) => true,
        Ok(false) => match ctx.repos.oauth_tokens.get_tokens(uuid, None).await {
            Ok(tokens) => !tokens.is_empty(),
            Err(e) => {
                warn!(
                    user_id = %uuid,
                    error = %e,
                    "athlete-record: oauth_tokens lookup failed — skipping the \
                     athlete-data layer rather than risk contradicting a true claim"
                );
                return None;
            }
        },
        Err(e) => {
            warn!(
                user_id = %uuid,
                error = %e,
                "athlete-record: provider_connections lookup failed — skipping the \
                 athlete-data layer rather than risk contradicting a true claim"
            );
            return None;
        }
    };

    // Without a provider there is nothing to fetch, and the window query would
    // be a guaranteed-empty round trip on every verified reply.
    if !connected {
        return Some(AthleteRecord::providerless());
    }

    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(ATHLETE_RECORD_WINDOW_DAYS);
    let activities = ctx
        .repos
        .activity_cache
        .get_cached_activities(uuid, &tenant_id, None, start, end, ATHLETE_RECORD_LIMIT)
        .await
        .unwrap_or_default();

    // The athlete's own zone, so a 21:00 session belongs to the day they
    // trained. A claim about "Tuesday" is checked against their Tuesday.
    let user_timezone = ctx
        .repos
        .users
        .get_global(uuid)
        .await
        .ok()
        .flatten()
        .and_then(|u| u.timezone);
    let zone = resolve_zone(user_timezone.as_deref());

    Some(AthleteRecord {
        has_provider: true,
        activities: activities
            .iter()
            .map(|a| {
                #[allow(clippy::cast_precision_loss)]
                let duration_min = (a.duration_seconds() as f64) / 60.0;
                RecordedActivity {
                    date: local_date(a.start_date(), zone),
                    sport: a.sport_type().clone(),
                    name: a.name().to_owned(),
                    distance_km: a.distance_meters().map(|m| m / 1000.0),
                    duration_min,
                    elevation_m: a.elevation_gain(),
                }
            })
            .collect(),
    })
}

/// How far back the athlete-data layer looks when matching a claim.
///
/// A coach discussing "last month" or "the past few weeks" is the common case;
/// a quarter covers those without pulling a whole history into a per-reply
/// check.
const ATHLETE_RECORD_WINDOW_DAYS: i64 = 90;

/// Cap on activities pulled for one verification pass.
///
/// The layer only needs the set of values a figure could match, and an athlete
/// with more than this in a quarter is not going to be adjudicated differently
/// by the tail.
const ATHLETE_RECORD_LIMIT: i64 = 500;

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
    if !config.personalized.enabled {
        return None;
    }
    let uuid = Uuid::parse_str(user_id).ok()?;
    let cageux = ctx.cageux_config_registry.current();
    let metrics =
        build_athlete_metrics(ctx.repos.as_ref(), &cageux.algorithms, tenant_id, uuid).await;
    if !metrics.is_usable() {
        return None;
    }
    let tolerance = config.personalized.tolerance_strategy();
    Some((metrics, tolerance))
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
    coach_id: Option<&str>,
    message_id: &str,
    pending: &[(ExtractedClaim, VerdictOutcome)],
) {
    for (claim, outcome) in pending {
        let params = InsertClaimVerdictParams {
            tenant_id,
            user_id,
            coach_id,
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
