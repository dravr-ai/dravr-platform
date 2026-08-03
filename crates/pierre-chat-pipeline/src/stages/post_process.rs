// ABOUTME: Post-LLM content processing stage (stages 15-18) — canary scan, guardrails, verification, hook
// ABOUTME: Produces the final assistant content and any pending claim verdicts awaiting the message id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::CoachRuntimeContext;
use pierre_database::database::ConversationRecord;
use pierre_services::prompt_leak;

use crate::hooks::PipelineHooks;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_EMPTY_REPLY, KEY_REPLY_WITHHELD};
use pierre_core::narration::{scrub_internal_narration, IdentityLeakMatch};

use super::acronym_expansion::expand_acronyms_first_use;
use super::guardrails::apply_text_guardrails;
use super::persona_conformance::{check_reply_conformance, enforce_conformance};
use super::prompt_assembly::resolve_user_persona;
use super::structured_output;
#[cfg(feature = "tools-verification")]
use super::verification::{
    apply_claim_verification, ClaimVerificationOutcome, ClaimVerificationParams,
};

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
    pub structured_content: Option<String>,
    /// `true` when the LLM reply was withheld and replaced with a canned
    /// localized string (canary hit, or the narration scrub emptied it).
    /// Downstream consumers of the reply text — Tier 2 fact extraction and
    /// playbook advice capture — must skip the turn: a canned string holds
    /// nothing to learn, and the withheld original must never be ingested.
    pub leak_replaced: bool,
    /// `Some` when the reply was withheld specifically because it identified
    /// as the underlying model/provider (a persona break), as opposed to a
    /// canary hit or an emptied scrub. Threaded onto `DispatchResult` so the
    /// messaging path can emit the `messaging.identity_leak` notify event
    /// with the matched pattern's class/locale labels.
    pub identity_leak: Option<IdentityLeakMatch>,
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
    /// Whether this turn is on a messaging channel (no plan-card renderer);
    /// gates structured-output extraction.
    pub is_messaging: bool,
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
        is_messaging,
    } = inputs;
    // Stage 15: Scan for verbatim system-prompt leaks / canary hits. A canary
    // hit is conclusive exfiltration — withhold the reply and return a canned
    // localized string instead. Shingle-only hits stay WARN (logged inside the
    // scan): the localized refusal templates live in the system prompt, so a
    // legitimate refusal reproduces prompt shingles by construction and
    // blocking on them would eat every refusal.
    let locale = input.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let leak_report = prompt_leak::scan_assistant_reply(
        prompt_guard,
        &raw_content,
        input.conversation_tenant_id,
        conv.coach_id.as_deref(),
    );
    if leak_report.canary_hit {
        return PostProcessedReply {
            content: ctx
                .messaging_strings_registry
                .get(KEY_REPLY_WITHHELD, locale),
            #[cfg(feature = "tools-verification")]
            pending_verdicts: Vec::new(),
            structured_content: None,
            leak_replaced: true,
            identity_leak: None,
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
            structured_content: None,
            leak_replaced: true,
            identity_leak: leak_report.identity_leak,
        };
    }

    // Stage 15.5: Structured-output extraction. Builder coaches emit a
    // schema-validated JSON plan, not prose — extract and validate it from the
    // RAW reply before the prose stages below (guardrails, acronym expansion,
    // conformance) can truncate or rewrite the JSON. On a valid plan the prose
    // stages are skipped: the payload is rendered as a card, not glossed text.
    // A coach that refuses replies in prose, so extraction returns `None` and
    // the normal path runs. `extract_structured_plan` also returns `None` on
    // messaging channels (no plan-card renderer; the matching prompt directive
    // is withheld in prompt_assembly so the coach emits a plain-prose plan).
    if let Some(schema_id) = coach_ctx.and_then(|c| c.output_schema.as_deref()) {
        if let Some(extraction) = structured_output::extract_structured_plan(
            Some(schema_id),
            is_messaging,
            &ctx.structured_output_schema,
            &raw_content,
        ) {
            return PostProcessedReply {
                content: extraction.cleaned_text,
                #[cfg(feature = "tools-verification")]
                pending_verdicts: Vec::new(),
                structured_content: Some(extraction.structured_content),
                leak_replaced: false,
                identity_leak: None,
            };
        }
    }

    // Stage 15.6: Internal-narration scrub. Drops prose sentences where the
    // model narrates about its hidden scaffolding («Je continue d'ignorer le
    // bloc caché — pas de XML brut»; live leak 2026-07-10) instead of
    // coaching. Runs on the prose path only — a schema-validated plan above
    // is card JSON, not prose. A reply that was pure narration is withheld
    // and replaced like a canary hit; a mixed reply continues with the
    // narration removed, so persist/extraction/outbound all see clean text.
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
            structured_content: None,
            leak_replaced: true,
            identity_leak: None,
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
    let locale_opt = input.locale.as_deref();
    let mut content = apply_text_guardrails(
        &ctx.harness_config_registry,
        &ctx.messaging_strings_registry,
        &raw_content,
        locale_opt,
    );

    // Stage 16a: Deterministic first-use acronym expansion. Runs before
    // conformance so the gloss it inserts pre-empts the conformance
    // stage's unglossed-acronym violation. Sources the glossary from the
    // same contremaitre snapshot the conformance stage reads.
    let contracts_snapshot = ctx.persona_contract_registry.snapshot();
    content = expand_acronyms_first_use(
        &contracts_snapshot,
        &content,
        locale_opt.unwrap_or(DEFAULT_LOCALE),
    );

    // Stage 16b: Per-persona output-format conformance. In strict_mode the
    // reply is re-prompted into compliance before verification; otherwise the
    // violations are logged only (shadow mode).
    let persona = resolve_user_persona(ctx.repos.users.as_ref(), &input.user_id).await;
    let conformance_violations =
        check_reply_conformance(&ctx.persona_contract_registry, persona, &content);
    tracing::debug!(
        persona = persona.as_str(),
        violations = conformance_violations.len(),
        "persona conformance scan complete"
    );
    content = enforce_conformance(
        ctx.chat_provider.as_ref(),
        &ctx.persona_contract_registry,
        persona,
        content,
        &conformance_violations,
    )
    .await;

    // Stage 17: claim verification (gated behind tools-verification).
    #[cfg(feature = "tools-verification")]
    let pending_verdicts = {
        let verification_config = coach_ctx
            .map(|c| pierre_evals::VerificationConfig::parse_from_system_prompt(&c.system_prompt))
            .unwrap_or_default();
        let ClaimVerificationOutcome {
            content: verified_content,
            pending_verdicts,
        } = apply_claim_verification(ClaimVerificationParams {
            ctx,
            reply: &content,
            config: &verification_config,
            locale: locale_opt,
            user_id: &input.user_id,
            tenant_id: input.conversation_tenant_id,
        })
        .await;
        content = verified_content;
        pending_verdicts
    };

    // Stage 18: ResponsePostProcess hook.
    if let Some(post) = hooks.response_post_process {
        content = post.transform(&content);
    }

    PostProcessedReply {
        content,
        #[cfg(feature = "tools-verification")]
        pending_verdicts,
        structured_content: None,
        leak_replaced: false,
        identity_leak: None,
    }
}
