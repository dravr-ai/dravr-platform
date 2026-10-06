// ABOUTME: The attribution line a coach reply owes when its turn handed the model Garmin device-sourced data
// ABOUTME: Appended by the platform to the delivered and persisted reply on every surface, never left to the model (carnet#521)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin attribution on AI insights.
//!
//! Garmin's API brand guidelines (combined or derived data): any output an AI
//! model derives in part from Garmin device-sourced data carries a Garmin
//! attribution, such as "Insights derived in part from Garmin device-sourced
//! data". intervals.icu's API terms bind Dravr to those guidelines for the
//! Garmin data it relays.
//!
//! Whether a turn drew on such data is a fact the platform holds, so the line
//! is the platform's. It reads the turn's provenance (carnet#769): every item
//! the provider-terms filters serve notes its attribution there, on the turn's
//! own task and on every executor the turn hands its accumulator to, the
//! Copilot loopback included ([`pierre_providers::ai_scope::Provenance`]).
//! When Garmin is among them, this stage appends the localized line to the
//! reply the athlete receives and the conversation keeps.

use std::collections::BTreeSet;

use pierre_contremaitre::messaging_strings::KEY_REPLY_GARMIN_ATTRIBUTION;
use pierre_core::constants::oauth_providers::GARMIN_ATTRIBUTION;
use pierre_providers::ai_scope;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

use super::reply_locale::resolve_banner_locale;
use crate::ChatPipelineContext;

/// Between the model's words and the attribution line.
const ATTRIBUTION_SEPARATOR: &str = "\n\n";

/// The attributions this turn's reply owes: those of everything the turn has
/// served so far, as its provenance records them; none outside a tracked turn.
#[must_use]
pub fn owed_attributions() -> BTreeSet<&'static str> {
    ai_scope::current_provenance()
        .map(|provenance| provenance.attributions())
        .unwrap_or_default()
}

/// Whether the reply is the model's answer, which the turn's reads informed:
/// not the platform's own reconnect, blocked-for-safety or confirmation text,
/// which no insight is derived into.
pub fn model_answered(result: &ToolLoopResult) -> bool {
    result.pending_provider_auth_required.is_none()
        && result.guardian_denied.is_none()
        && result.guardian_confirm.is_none()
}

/// `reply` with the Garmin attribution line appended when `owed` carries
/// Garmin and the model `answered`, in the reply's own language (else
/// `locale`); unchanged otherwise, for an empty reply, or when the reply
/// already ends with the line.
pub fn attribute_reply(
    ctx: &ChatPipelineContext,
    reply: String,
    owed: &BTreeSet<&'static str>,
    answered: bool,
    locale: &str,
) -> String {
    if !answered || !owed.contains(GARMIN_ATTRIBUTION) || reply.trim().is_empty() {
        return reply;
    }
    let line = ctx.messaging_strings_registry.get(
        KEY_REPLY_GARMIN_ATTRIBUTION,
        &resolve_banner_locale(&reply, locale),
    );
    with_attribution(reply, &line)
}

/// `reply` followed by `line`, once.
fn with_attribution(reply: String, line: &str) -> String {
    if line.is_empty() || reply.trim_end().ends_with(line) {
        return reply;
    }
    format!("{reply}{ATTRIBUTION_SEPARATOR}{line}")
}

#[cfg(test)]
mod tests {
    use super::with_attribution;

    const LINE: &str = "Insights derived in part from Garmin device-sourced data.";

    #[test]
    fn the_line_follows_the_reply_once() {
        let once = with_attribution("Keep Monday easy.".to_owned(), LINE);
        assert_eq!(once, format!("Keep Monday easy.\n\n{LINE}"));
        assert_eq!(with_attribution(once.clone(), LINE), once, "never twice");
        assert_eq!(
            with_attribution("Keep Monday easy.".to_owned(), ""),
            "Keep Monday easy.",
            "a missing string adds nothing"
        );
    }
}
