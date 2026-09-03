// ABOUTME: Post-LLM content processing stage (stages 15-18) — canary scan, guardrails, verification, hook
// ABOUTME: Produces the final assistant content and any pending claim verdicts awaiting the message id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::{CoachRuntimeContext, CoachingPersona};
use pierre_database::database::ConversationRecord;
use pierre_services::prompt_leak;
use tracing::{info, warn};
use uuid::Uuid;

use crate::envelope::VerdictChip;
use crate::hooks::PipelineHooks;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

use pierre_contremaitre::messaging_strings::{KEY_EMPTY_REPLY, KEY_REPLY_WITHHELD};
use pierre_contremaitre::persona_contracts::PersonaContractsSnapshot;
use pierre_core::narration::{
    scrub_internal_narration, scrub_ungrounded_data_appeals, IdentityLeakMatch,
};

use super::acronym_expansion::expand_acronyms_first_use;
use super::guardrails::apply_text_guardrails;
use super::persona_conformance::{
    apply_isolation_redaction, check_reply_conformance, enforce_conformance, RosterScope,
};
use super::prompt_assembly::resolve_user_persona;
use super::structured_output;
#[cfg(feature = "tools-verification")]
use super::verification::{
    apply_claim_verification, ClaimVerificationOutcome, ClaimVerificationParams,
};
use super::viz_blocks;

/// Aggregates the outputs of [`post_process_assistant_reply`] so the
/// caller can persist the assistant message first and then link any
/// claim verdicts to the resulting `message_id`.
///
/// The `pending_verdicts` field is compiled out when
/// `tools-verification` is disabled to keep the struct free of
/// `pierre_evals` types.
pub(crate) struct PostProcessedReply {
    /// Final assistant content ready to be persisted and returned.
    pub content: String,
    /// Verdicts waiting for their assistant `message_id`. Always empty
    /// when the verification feature is disabled.
    #[cfg(feature = "tools-verification")]
    pub pending_verdicts: Vec<(pierre_evals::ExtractedClaim, pierre_evals::VerdictOutcome)>,
    /// Schema-validated structured payload (e.g. a workout plan) extracted
    /// from a builder-coach reply. `Some` only when the coach declares an
    /// `output_schema` and the reply validates against it.
    /// Ordered visual blocks lifted out of the reply's prose, JSON-encoded.
    /// `Some` only when at least one fenced `dravr-viz` block validated; the
    /// reply text then carries a positional marker where each block sat.
    pub content_blocks: Option<String>,
    /// `true` when the LLM reply was withheld and replaced with a canned
    /// localized string (canary hit, or the narration scrub emptied it).
    /// Downstream consumers of the reply text — Tier 2 fact extraction and
    /// playbook advice capture — must skip the turn: a canned string holds
    /// nothing to learn, and the withheld original must never be ingested.
    pub leak_replaced: bool,
    /// `Some` when the reply was withheld specifically because it identified
    /// as the underlying model/provider (a persona break), as opposed to a
    /// canary hit or an emptied scrub. Threaded onto
    /// [`crate::TurnTelemetry`] so the messaging surface can emit the
    /// `messaging.identity_leak` notify event with the matched pattern's
    /// class/locale labels.
    pub identity_leak: Option<IdentityLeakMatch>,
    /// Flagged claims to attach to the reply as chips. Non-empty only when the
    /// surface renders chips, which is exactly when the verification stage left
    /// the caveat banner out of `content`.
    pub verdict_chips: Vec<VerdictChip>,
}

/// Borrowed inputs to [`post_process_assistant_reply`], bundled to stay within
/// the argument-count lint. `raw_content` (owned/consumed) and `hooks` stay
/// separate arguments.
pub(crate) struct PostProcessInputs<'a> {
    pub ctx: &'a ChatPipelineContext,
    pub input: &'a TurnInput,
    pub conv: &'a ConversationRecord,
    pub coach_ctx: Option<&'a CoachRuntimeContext>,
    pub prompt_guard: &'a prompt_leak::PromptGuard,
    /// What the turn's surface can render, plus its resolved locale. Read for
    /// the plan-card capability that gates structured-output extraction, the
    /// transport character ceiling the guardrails stage enforces, and the
    /// locale every canned reply renders in.
    pub profile: &'a SurfaceProfile,
    /// Names of the tools that actually ran this turn. A visual block claiming
    /// a `source_tool` outside this set is rejected, which is what makes the
    /// attribution verified rather than asserted.
    pub tools_called: &'a [String],
    /// Whether this turn stands on evidence gathered for it — a tool ran, or
    /// the prefetch injected an activity block.
    ///
    /// `false` means the reply was produced from conversation history alone, and
    /// the appeal scrub applies: the coach may still answer, it just cannot cite
    /// a lookup it did not perform (registre#202).
    pub turn_was_grounded: bool,
    /// The model this turn actually ran on.
    ///
    /// Needed because the persona repair re-prompts the SAME provider. Without
    /// it the repair sends no model, `resolve_model(None)` falls back to the
    /// env default, and on a provider that pins a model per subprocess — the
    /// ACP pool does — a mismatch discards the warm subprocess and pays a cold
    /// spawn on every repair turn.
    pub active_model: &'a str,
}

/// Resolve the coach's roster so the conformance stage can tell a cited athlete
/// apart from a stranger.
///
/// Queried only when the persona's contract sets `require_tenant_isolation` —
/// every other persona would pay a roster lookup that no rule reads. A failed
/// or empty lookup yields `None`, and the check fails CLOSED on it: citations
/// that cannot be verified are treated as foreign and redacted.
async fn resolve_roster_scope(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    snapshot: &PersonaContractsSnapshot,
    persona: CoachingPersona,
) -> Option<RosterScope> {
    if !snapshot
        .contract(persona)
        .is_some_and(|c| c.require_tenant_isolation)
    {
        return None;
    }
    let coach_id = Uuid::parse_str(&input.user_id).ok()?;
    match ctx
        .repos
        .roster
        .list_athletes_for_coach(coach_id, input.conversation_tenant_id)
        .await
    {
        Ok(assignments) => Some(RosterScope::from_athlete_ids(
            assignments.iter().map(|a| a.athlete_user_id.to_string()),
        )),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "roster lookup failed; tenant-isolation conformance will fail open"
            );
            None
        }
    }
}

/// Lift inline visual blocks out of a reply, returning the marker-bearing prose
/// and the encoded blocks.
///
/// Runs on the RAW reply for the same reason the plan extraction above does:
/// the prose stages that follow would rewrite or truncate the JSON. What
/// continues down the prose path is text with a positional marker where each
/// block sat, so guardrails, acronym expansion and conformance all operate on
/// something a human would read.
///
/// Skipped on messaging — there is no block renderer there, and stripping the
/// fences would leave markers pointing at nothing. The matching prompt
/// permission is withheld there too, so a messaging coach never emits one.
/// Stage 16a then 16b: gloss first-use acronyms, then enforce the persona's
/// output-format contract.
///
/// Ordered, not merely adjacent: the gloss must land before the conformance
/// scan, or the scan reports the unglossed-acronym violation the gloss just
/// pre-empted. Both read the same contremaitre snapshot for that reason.
async fn apply_style_stages(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    content: String,
    locale: &str,
    active_model: &str,
) -> String {
    let contracts_snapshot = ctx.persona_contract_registry.snapshot();
    let content = expand_acronyms_first_use(&contracts_snapshot, &content, locale);

    // In strict_mode the reply is re-prompted into compliance before
    // verification; otherwise the violations are logged only (shadow mode).
    let persona = resolve_user_persona(ctx.repos.users.as_ref(), &input.user_id).await;
    let roster = resolve_roster_scope(ctx, input, &contracts_snapshot, persona).await;
    let conformance_violations = check_reply_conformance(
        &ctx.persona_contract_registry,
        persona,
        &content,
        roster.as_ref(),
    );
    tracing::debug!(
        persona = persona.as_str(),
        violations = conformance_violations.len(),
        "persona conformance scan complete"
    );
    // Leak repair before style repair: an isolation violation is cut
    // deterministically; only style violations may reach the LLM rewrite.
    let content = apply_isolation_redaction(
        &ctx.messaging_strings_registry,
        persona,
        content,
        &conformance_violations,
        roster.as_ref(),
        locale,
    );
    enforce_conformance(
        ctx.chat_provider.as_ref(),
        &ctx.persona_contract_registry,
        persona,
        content,
        &conformance_violations,
        active_model,
    )
    .await
}

/// One bounded re-ask for a reply whose blocks the schema refused.
///
/// `Some` only for a repair that is strictly better than what we already had:
/// it recovered at least one block *and* left fewer refusals behind. Anything
/// else — no provider, a failed or empty completion, a repair that traded one
/// refusal for another — yields `None` and the original extraction stands, so
/// this can only add a chart, never cost the athlete prose they would have got.
async fn repaired_extraction(
    ctx: &ChatPipelineContext,
    granted: &[String],
    tools_called: &[String],
    raw_content: &str,
    current: &viz_blocks::VizExtraction,
    active_model: &str,
) -> Option<viz_blocks::VizExtraction> {
    if current.refusals.is_empty() {
        return None;
    }
    let provider = ctx.chat_provider.as_ref()?;
    let repaired =
        viz_blocks::repair_refused_blocks(provider, raw_content, &current.refusals, active_model)
            .await?;
    let second = viz_blocks::extract_viz_blocks(
        &ctx.structured_output_schemas,
        granted,
        tools_called,
        &repaired,
    )?;
    if second.blocks.len() > current.blocks.len() && second.refusals.len() < current.refusals.len()
    {
        info!(
            recovered = second.blocks.len() - current.blocks.len(),
            still_refused = second.refusals.len(),
            "viz-blocks: repair re-ask recovered a block the schema had refused"
        );
        return Some(second);
    }
    info!(
        still_refused = second.refusals.len(),
        "viz-blocks: repair re-ask did not improve on the original; keeping it"
    );
    None
}

async fn lift_viz_blocks(
    ctx: &ChatPipelineContext,
    coach_ctx: Option<&CoachRuntimeContext>,
    tools_called: &[String],
    raw_content: String,
    active_model: &str,
) -> (String, Option<String>, usize) {
    // Extraction is channel-agnostic. Messaging used to short-circuit here,
    // when a chart had nowhere to go on a channel that cannot render one
    // inline; it has somewhere now, because the egress mints a signed image URL
    // per block and sends it as media. Which channels get pixels rather than
    // the prose fallback is the egress's decision, not this stage's.
    // A coach with no `visuals:` grant is never shown the contract, so a fence
    // in its reply is not something we asked for. Extracting it anyway would
    // make the grant advisory; refusing keeps it a permission. The fence stays
    // in the text, visible, rather than being silently swallowed.
    // Same rule the prompt-assembly stage used to decide whether to teach the
    // contract. If the two ever disagree, a coach is told it may draw and then
    // has its block refused — which is precisely the raw-JSON reply this stage
    // exists to prevent.
    let granted = viz_blocks::granted_visuals(coach_ctx.map(|c| c.visuals.as_slice()));
    if granted.is_empty() {
        return (raw_content, None, 0);
    }
    let granted = granted.as_slice();
    let Some(mut extraction) = viz_blocks::extract_viz_blocks(
        &ctx.structured_output_schemas,
        granted,
        tools_called,
        &raw_content,
    ) else {
        return (raw_content, None, 0);
    };

    // A refused block is the failure the athlete feels: they asked for a chart
    // and the prose arrives without one, with nothing admitting a visual was
    // withheld. The refusals now name the offending field, so the model can be
    // handed something it can act on — one re-ask, fail-open.
    if let Some(better) = repaired_extraction(
        ctx,
        granted,
        tools_called,
        &raw_content,
        &extraction,
        active_model,
    )
    .await
    {
        extraction = better;
    }
    // Re-encoding cannot realistically fail for values that were just parsed,
    // but if it did, keeping the marker text without the blocks would leave the
    // athlete reading a bare `⟦viz:0⟧`.
    let count = extraction.blocks.len();
    // Every fence was refused: the stage still hands back the cleaned text (the
    // fences are gone from it), but there is nothing to store. Serializing the
    // empty vec would persist a literal "[]" on the message and have the egress
    // negotiate media for zero blocks.
    if count == 0 {
        return (extraction.text, None, 0);
    }
    match serde_json::to_string(&extraction.blocks) {
        Ok(json) => (extraction.text, Some(json), count),
        Err(e) => {
            warn!(error = %e, "viz-blocks: extracted blocks failed to re-encode; leaving the reply as-is");
            (raw_content, None, 0)
        }
    }
}

/// Drop sentences that cite fetched data on a turn where nothing was fetched.
///
/// Live 2026-09-02, on a zero-tool turn and immediately after the athlete had
/// corrected it for the third time: «Roster data confirme: Date ride était bien
/// lundi». The coach turned the athlete's own correction into evidence against
/// him. On a grounded turn the same sentence is true, so this returns the reply
/// untouched (registre#202).
///
/// A reply that was *nothing but* appeals is left alone rather than emptied:
/// the narration scrub downstream owns the withhold-and-replace path, and two
/// stages competing for it is how a reply goes silently blank.
fn drop_ungrounded_data_appeals(
    raw_content: String,
    turn_was_grounded: bool,
    input: &TurnInput,
) -> String {
    if turn_was_grounded {
        return raw_content;
    }
    let appeal = scrub_ungrounded_data_appeals(&raw_content);
    if !appeal.fired() || appeal.cleaned.is_empty() {
        return raw_content;
    }
    tracing::warn!(
        tenant_id = %input.conversation_tenant_id,
        sentences_removed = appeal.removed,
        "appeal to fetched data scrubbed from a turn that fetched nothing"
    );
    appeal.cleaned
}

/// Run post-LLM content processing over the raw assistant reply.
///
/// Owns pipeline stages 15 through 18: canary scan, text guardrails,
/// claim verification, and the channel-specific
/// [`ResponsePostProcess`](super::super::hooks::ResponsePostProcess)
/// hook. Returns the content string alongside any pending verdicts so
/// the caller can persist them with the assistant `message_id` after
/// the message write succeeds — emitting verdicts before the message
/// exists would leave orphan rows if the message write failed.
pub(crate) async fn post_process_assistant_reply(
    inputs: PostProcessInputs<'_>,
    raw_content: String,
    hooks: &PipelineHooks<'_>,
) -> PostProcessedReply {
    let PostProcessInputs {
        ctx,
        input,
        conv,
        coach_ctx,
        prompt_guard,
        profile,
        tools_called,
        turn_was_grounded,
        active_model,
    } = inputs;
    // Stage 15: Scan for verbatim system-prompt leaks / canary hits. A canary
    // hit is conclusive exfiltration — withhold the reply and return a canned
    // localized string instead. Shingle-only hits stay WARN (logged inside the
    // scan): the localized refusal templates live in the system prompt, so a
    // legitimate refusal reproduces prompt shingles by construction and
    // blocking on them would eat every refusal.
    let locale = profile.locale.as_str();
    // Keyed on the coach that answered this turn, the same salt the canary was
    // minted with in prompt assembly.
    let leak_report = prompt_leak::scan_assistant_reply(
        prompt_guard,
        &raw_content,
        input.conversation_tenant_id,
        input.turn_coach_id(conv),
    );
    if leak_report.canary_hit {
        return PostProcessedReply {
            content: ctx
                .messaging_strings_registry
                .get(KEY_REPLY_WITHHELD, locale),
            #[cfg(feature = "tools-verification")]
            pending_verdicts: Vec::new(),
            content_blocks: None,
            leak_replaced: true,
            identity_leak: None,
            verdict_chips: Vec::new(),
        };
    }

    // Stage 15.4: Model-identity leak. Production messaging runs the coach
    // through GitHub Copilot CLI, which owns the true system slot, so the model
    // periodically answers as itself (« I'm GitHub Copilot CLI, a terminal-based
    // coding assistant » reached a live Telegram user on 2026-07-22). That is a
    // whole persona break, not salvageable sentence-by-sentence — withhold the
    // entire reply like a canary hit. `leak_replaced = true` gates Tier-2
    // learning and stamps the persisted row with `WITHHELD_REPLY_FINISH_REASON`,
    // which is what keeps the apology out of the coach turn's replayed history
    // — the row itself IS persisted (the athlete saw it), contrary to what this
    // comment claimed before 2026-08-02: WITHHELD_REPLY_TRANSCRIPT_MARKER only
    // ever reaches the fact-extraction prompt, never the database. The
    // detection is logged (alertable) inside `scan_assistant_reply`.
    if leak_report.identity_leak.is_some() {
        return PostProcessedReply {
            content: ctx
                .messaging_strings_registry
                .get(KEY_REPLY_WITHHELD, locale),
            #[cfg(feature = "tools-verification")]
            pending_verdicts: Vec::new(),
            content_blocks: None,
            leak_replaced: true,
            identity_leak: leak_report.identity_leak,
            verdict_chips: Vec::new(),
        };
    }

    // Stage 15.5: Structured-output extraction. Builder coaches emit a
    // schema-validated JSON plan, not prose — extract and validate it from the
    // RAW reply before the prose stages below (guardrails, acronym expansion,
    // conformance) can truncate or rewrite the JSON. On a valid plan the prose
    // stages are skipped: the payload is rendered as a card, not glossed text.
    // A coach that refuses replies in prose, so extraction returns `None` and
    // the normal path runs.
    //
    // The plan leaves here as a `workout_plan` **block**, not on a field of its
    // own. It used to ride `structured_content` with a separate validator and a
    // separate extractor while charts and tables rode `content_blocks` — two
    // rails for one idea. One rail means a client learns a single shape, and a
    // reply could carry a plan and a chart together if a coach ever wanted it.
    if let Some(schema_id) = coach_ctx.and_then(|c| c.output_schema.as_deref()) {
        if let Some(extraction) = structured_output::extract_structured_plan(
            Some(schema_id),
            profile.render.blocks.workout_plan_card,
            &ctx.structured_output_schemas,
            &raw_content,
        ) {
            return PostProcessedReply {
                content: extraction.cleaned_text,
                #[cfg(feature = "tools-verification")]
                pending_verdicts: Vec::new(),
                content_blocks: structured_output::plan_as_block(
                    &extraction.structured_content,
                    schema_id,
                ),
                leak_replaced: false,
                identity_leak: None,
                verdict_chips: Vec::new(),
            };
        }
    }

    // Stage 15.55: Inline visual blocks.
    let (raw_content, content_blocks, block_count) =
        lift_viz_blocks(ctx, coach_ctx, tools_called, raw_content, active_model).await;

    // Stage 15.6: Internal-narration scrub. Drops prose sentences where the
    // model narrates about its hidden scaffolding («Je continue d'ignorer le
    // bloc caché — pas de XML brut»; live leak 2026-07-10) instead of
    // coaching. Runs on the prose path only — a schema-validated plan above
    // is card JSON, not prose. A reply that was pure narration is withheld
    // and replaced like a canary hit; a mixed reply continues with the
    // narration removed, so persist/extraction/outbound all see clean text.
    // Stage 15.55b: an ungrounded turn may not appeal to data as its authority.
    let raw_content = drop_ungrounded_data_appeals(raw_content, turn_was_grounded, input);

    let scrub = scrub_internal_narration(&raw_content);
    if scrub.fired() {
        tracing::warn!(
            tenant_id = %input.conversation_tenant_id,
            sentences_removed = scrub.removed,
            emptied = scrub.cleaned.is_empty(),
            "internal narration scrubbed from assistant reply"
        );
    }
    if scrub.fired() && scrub.cleaned.is_empty() {
        return PostProcessedReply {
            content: ctx.messaging_strings_registry.get(KEY_EMPTY_REPLY, locale),
            #[cfg(feature = "tools-verification")]
            pending_verdicts: Vec::new(),
            content_blocks: None,
            leak_replaced: true,
            identity_leak: None,
            verdict_chips: Vec::new(),
        };
    }
    // An untouched reply passes through byte-identical; only a fired scrub
    // swaps in the cleaned text.
    let raw_content = if scrub.fired() {
        scrub.cleaned
    } else {
        raw_content
    };

    // Stage 16: Tier 6 text guardrails.
    let mut content = apply_text_guardrails(
        &ctx.harness_config_registry,
        &ctx.messaging_strings_registry,
        &raw_content,
        locale,
    );

    // Stages 16a-16b: acronym gloss, then per-persona output-format conformance.
    content = apply_style_stages(ctx, input, content, locale, active_model).await;

    // Stage 17: claim verification (gated behind tools-verification).
    #[cfg(not(feature = "tools-verification"))]
    let verdict_chips: Vec<VerdictChip> = Vec::new();
    #[cfg(feature = "tools-verification")]
    let (pending_verdicts, verdict_chips) = {
        let verification_config = coach_ctx
            .map(|c| pierre_evals::VerificationConfig::parse_from_system_prompt(&c.system_prompt))
            .unwrap_or_default();
        let ClaimVerificationOutcome {
            content: verified_content,
            pending_verdicts,
            chips,
        } = apply_claim_verification(ClaimVerificationParams {
            ctx,
            renders_chips: profile.render.blocks.verdict_chips,
            reply: &content,
            config: &verification_config,
            locale,
            user_id: &input.user_id,
            tenant_id: input.conversation_tenant_id,
        })
        .await;
        content = verified_content;
        (pending_verdicts, chips)
    };

    // Stage 18: ResponsePostProcess hook.
    if let Some(post) = hooks.response_post_process {
        content = post.transform(&content);
    }

    // Blocks survive only if their markers did. Guardrails, the too-long
    // truncation and the verification fallback each replace the reply wholesale
    // without knowing about blocks; shipping the blocks anyway would render a
    // chart underneath a refusal, positioned by a marker that no longer exists.
    let content_blocks = content_blocks.filter(|_| {
        let intact = viz_blocks::markers_intact(&content, block_count);
        if !intact {
            warn!(
                block_count,
                "viz-blocks: reply was rewritten after extraction and lost its markers; dropping the blocks"
            );
        }
        intact
    });

    PostProcessedReply {
        content,
        #[cfg(feature = "tools-verification")]
        pending_verdicts,
        content_blocks,
        leak_replaced: false,
        identity_leak: None,
        verdict_chips,
    }
}
