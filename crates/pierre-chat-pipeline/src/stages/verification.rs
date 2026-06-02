// ABOUTME: Tier 5.5 claim verification stage — runs the bullshit detector pipeline over the assistant reply
// ABOUTME: Provides apply_claim_verification — emits ClaimVerdict rows for evidence-strength chips
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 5.5 claim verification.
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

use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::{ExtractedClaim, VerdictOutcome, VerificationConfig, VerificationFallback};
use pierre_memory::claims::{ClaimCategory, ClaimStatus};

use crate::ChatPipelineContext;
use pierre_contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_VERIFICATION_BLOCK_FALLBACK, KEY_VERIFICATION_WARN_SUFFIX,
};
use pierre_core::models::TenantId;
use pierre_runtime_context::DataContext;
use pierre_services::claim_verification::resolve_corpus;
use pierre_services::claim_verification::verify_reply_with_config_and_judge;

/// Localizes the verification warn / block-fallback strings.
///
/// The Tier 5.5 banner is appended verbatim to the LLM's reply, so the
/// banner's language must match the reply's language — otherwise an
/// English session ends with a French postscript (or vice versa).
///
/// Resolution order (returns first match):
/// 1. **Reply text via whatlang** — long replies (≥ a few sentences)
///    detect reliably even for casual conversational tone, which is the
///    case the per-turn locale heuristic in `messaging_ingress` misses
///    (a 4-word user question can't be detected, but the 200-word reply
///    can).
/// 2. **Caller-supplied `locale`** — usually the per-turn locale that
///    upstream code attempted to resolve from the user's input or
///    `users.locale`. Honored verbatim when set.
/// 3. **`DEFAULT_LOCALE`** — last resort.
pub(crate) fn resolve_banner_locale(reply: &str, locale: Option<&str>) -> String {
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
    locale.unwrap_or(DEFAULT_LOCALE).to_owned()
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
        .filter_map(|(claim, outcome)| match outcome.status {
            ClaimStatus::Contradicted => Some((claim.text.as_str(), true)),
            ClaimStatus::Unsupported if claim.category != ClaimCategory::TrainingPrescription => {
                Some((claim.text.as_str(), false))
            }
            _ => None,
        })
        .collect()
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
            "Tier 5.5 truncated the flagged-claim warning list"
        );
    }
    flagged
        .iter()
        .map(|(claim, _)| format!("- {}", claim.trim()))
        .collect()
}

/// Inputs to [`apply_claim_verification`].
///
/// Bundles the Tier 5.5 turn context so the function signature stays under
/// clippy's `too_many_arguments` ceiling. All fields are borrowed from
/// caller-owned state — no ownership transfer, no cloning.
pub struct ClaimVerificationParams<'a> {
    /// Pipeline context providing the registries and corpus.
    pub ctx: &'a ChatPipelineContext,
    /// Assistant reply text to scan.
    pub reply: &'a str,
    /// Parsed verification config (from the coach's prompt frontmatter).
    pub config: &'a VerificationConfig,
    /// Resolved locale for Warn/Block fallback strings, `None` → default.
    pub locale: Option<&'a str>,
}

/// Result of running Tier 5.5 verification on an assistant reply.
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
}

/// Run the bullshit detector over the finalized assistant reply.
///
/// Computes verdicts and applies the coach's
/// [`VerificationConfig::fallback_behavior`] to the reply, but defers
/// persisting verdicts. The caller is expected to write the assistant message
/// first and then invoke [`persist_pending_verdicts`] with the resulting
/// `message_id`. This keeps the audit row linked to the message that
/// produced it and avoids orphan verdicts when the message write fails.
pub async fn apply_claim_verification(
    params: ClaimVerificationParams<'_>,
) -> ClaimVerificationOutcome {
    let ClaimVerificationParams {
        ctx,
        reply,
        config,
        locale,
    } = params;
    let locale = resolve_banner_locale(reply, locale);
    let locale = locale.as_str();

    if !config.enabled {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
        };
    }

    let corpus = resolve_corpus(&ctx.evidence_registry);
    // Layer 5 LLM judge runs only when a provider is configured; otherwise the
    // pipeline stays fully deterministic and inconclusive claims settle on the
    // evidence layer's verdict.
    let judge = ctx.llm_provider.as_deref();
    let verdicts = match verify_reply_with_config_and_judge(reply, config, &corpus, judge).await {
        Ok(verdicts) => verdicts,
        Err(e) => {
            tracing::warn!(error = %e, "Tier 5.5 verification failed — skipping claim verdicts");
            return ClaimVerificationOutcome {
                content: reply.to_owned(),
                pending_verdicts: Vec::new(),
            };
        }
    };
    if verdicts.is_empty() {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
        };
    }

    let problems = actionable_problems(&verdicts);

    let content = if problems.is_empty() {
        reply.to_owned()
    } else {
        match config.fallback_behavior {
            VerificationFallback::Warn => {
                let bullets = warning_bullets(&problems, reply);
                if bullets.is_empty() {
                    reply.to_owned()
                } else {
                    let header = ctx
                        .messaging_strings_registry
                        .get(KEY_VERIFICATION_WARN_SUFFIX, locale);
                    let body = bullets.join("\n");
                    format!("{reply}\n\n---\n{header}\n{body}")
                }
            }
            VerificationFallback::Silent => reply.to_owned(),
            VerificationFallback::Block => {
                tracing::warn!(
                    flagged_claims = problems.len(),
                    "Tier 5.5 block fallback fired — replacing reply"
                );
                ctx.messaging_strings_registry
                    .get(KEY_VERIFICATION_BLOCK_FALLBACK, locale)
            }
        }
    };

    ClaimVerificationOutcome {
        content,
        pending_verdicts: verdicts,
    }
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
