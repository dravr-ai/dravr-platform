// ABOUTME: Suggestion postbacks — the opaque value a tapped suggestion sends, resolved before the turn is dispatched
// ABOUTME: Makes a tap measurable (suggestion.shown / .tapped, InputSource::UseCase) and keeps the id out of the transcript
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Suggestion postbacks (carnet#828).
//!
//! A welcome's starter used to post its whole question as its postback, so a
//! tap was the same text as typing that question and nothing could say
//! whether suggestions are used at all. A suggestion now posts an opaque value
//! that `resolve` turns back into its words before the turn is dispatched:
//! the athlete's bubble and the transcript carry the question, the turn is
//! tagged [`InputSource::UseCase`](pierre_core::models::InputSource), and
//! `suggestion.tapped` records which suggestion, from which slot.
//!
//! The grammar is `ex:<position>:<index>`: the agent's Example Input at
//! `index` in `starter_samples` order, shown in slot `position`. The two
//! numbers coincide while a welcome offers its first three examples in order;
//! they are kept apart so a row written today still resolves once ranked use
//! cases take the slots ahead of the examples.
//!
//! A value parses only when it is exactly that shape, so a question an athlete
//! types — and every slash-command button, whose value starts with `/` — is
//! never mistaken for one.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::RepositoryRegistry;
use tracing::info;
use uuid::Uuid;

use crate::agent_welcome::{localized_agent, starter_samples};

/// The `surface` an agent welcome's starters are reported under.
pub(crate) const SURFACE_AGENT_WELCOME: &str = "agent_welcome";

const EXAMPLE_PREFIX: &str = "ex:";

/// Where a suggestion's words came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionSource {
    /// The agent's own authored Example Inputs, the same for every athlete.
    Static,
}

impl SuggestionSource {
    /// The analytics value for this source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
        }
    }
}

/// A suggestion's postback, parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Postback {
    /// The bound agent's Example Input at `index`, in [`starter_samples`]
    /// order, offered in slot `position`.
    Example {
        /// 0-based slot the suggestion was shown in.
        position: usize,
        /// 0-based index into the agent's normalized Example Inputs.
        index: usize,
    },
}

impl Postback {
    /// The value a control carries for this suggestion.
    #[must_use]
    pub fn encode(self) -> String {
        match self {
            Self::Example { position, index } => format!("{EXAMPLE_PREFIX}{position}:{index}"),
        }
    }

    /// Parse a turn's text as a suggestion postback; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let rest = text.trim().strip_prefix(EXAMPLE_PREFIX)?;
        let (position, index) = rest.split_once(':')?;
        Some(Self::Example {
            position: parse_slot(position)?,
            index: parse_slot(index)?,
        })
    }

    /// The suggestion's identity in analytics: `ex:<index>`, the same for a
    /// given example whichever slot it was offered in.
    #[must_use]
    pub fn use_case(self) -> String {
        match self {
            Self::Example { index, .. } => format!("{EXAMPLE_PREFIX}{index}"),
        }
    }

    /// The slot the suggestion was shown in.
    #[must_use]
    pub const fn position(self) -> usize {
        match self {
            Self::Example { position, .. } => position,
        }
    }

    /// Where the suggestion's words came from.
    #[must_use]
    pub const fn source(self) -> SuggestionSource {
        match self {
            Self::Example { .. } => SuggestionSource::Static,
        }
    }
}

/// Whether a control's `value` is a suggestion postback, whose text fallback
/// shows the label alone: the value means nothing typed back.
#[must_use]
pub fn is_postback(value: &str) -> bool {
    Postback::parse(value).is_some()
}

/// A slot or index: ASCII digits only, so `+1` or ` 1` never parse.
fn parse_slot(digits: &str) -> Option<usize> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// What [`resolve`] made of a turn's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Not a suggestion postback: the turn proceeds with the text as sent.
    NotAPostback,
    /// The postback stands for `content`, which the turn answers instead.
    Resolved {
        /// The suggestion's words, as the athlete would have typed them.
        content: String,
        /// The postback that was tapped.
        postback: Postback,
    },
    /// A postback that no longer resolves — the thread lost its agent, or the
    /// agent's examples changed since the suggestion was posted.
    Unresolvable {
        /// The postback that was tapped.
        postback: Postback,
    },
}

/// Who tapped, where, for [`resolve`] and the analytics it emits.
pub(crate) struct TapContext<'a> {
    /// The thread the tap was sent in, when it could be read.
    pub conversation: Option<&'a ConversationRecord>,
    /// The athlete who tapped.
    pub user_id: Uuid,
    /// The athlete's own tenant, where the thread's agent resolves for them.
    pub agent_tenant_id: TenantId,
    /// Locale the suggestion's words are read back in.
    pub locale: &'a str,
}

/// Resolve `text` when it is a suggestion postback.
///
/// An example resolves against the agent the thread is bound to, read in
/// `tap.locale` and normalized exactly as the welcome normalized it, so an
/// index points at the same words the athlete saw.
///
/// # Errors
///
/// Returns the repository error when the agent or its translation cannot be
/// read. A transient failure is not reported as a suggestion gone stale.
pub(crate) async fn resolve(
    repos: &RepositoryRegistry,
    tap: &TapContext<'_>,
    text: &str,
) -> AppResult<Resolution> {
    let Some(postback) = Postback::parse(text) else {
        return Ok(Resolution::NotAPostback);
    };
    let Postback::Example { index, .. } = postback;
    let Some(agent_id) = tap.conversation.and_then(|c| c.agent_id.as_deref()) else {
        return Ok(Resolution::Unresolvable { postback });
    };
    let Some((_, localized)) = localized_agent(
        repos,
        agent_id,
        tap.user_id,
        tap.agent_tenant_id,
        tap.locale,
    )
    .await?
    else {
        return Ok(Resolution::Unresolvable { postback });
    };
    Ok(starter_samples(&localized.sample_prompts)
        .into_iter()
        .nth(index)
        .map_or(Resolution::Unresolvable { postback }, |content| {
            Resolution::Resolved { content, postback }
        }))
}

/// Who a suggestion event is about and where it happened.
pub(crate) struct SuggestionEvent<'a> {
    /// The athlete the suggestion was offered to.
    pub user_id: &'a str,
    /// Tenant of the conversation the suggestion lives in.
    pub tenant_id: TenantId,
    /// The placement: [`SURFACE_AGENT_WELCOME`].
    pub surface: &'a str,
    /// The chat surface, as [`crate::SurfaceId::as_str`] names it.
    pub channel: &'a str,
}

/// Record that `postback` was posted into a thread.
pub(crate) fn emit_shown(event: &SuggestionEvent<'_>, postback: Postback) {
    info!(
        target: "notify",
        event = "suggestion.shown",
        user_id = event.user_id,
        tenant_id = %event.tenant_id,
        use_case = %postback.use_case(),
        source = postback.source().as_str(),
        position = postback.position(),
        surface = event.surface,
        channel = event.channel,
        "suggestion shown"
    );
}

/// Record that the athlete tapped `postback` and it resolved.
pub(crate) fn emit_tapped(event: &SuggestionEvent<'_>, postback: Postback) {
    info!(
        target: "notify",
        event = "suggestion.tapped",
        user_id = event.user_id,
        tenant_id = %event.tenant_id,
        use_case = %postback.use_case(),
        source = postback.source().as_str(),
        position = postback.position(),
        surface = event.surface,
        channel = event.channel,
        "suggestion tapped"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_example_postback_round_trips() {
        let postback = Postback::Example {
            position: 2,
            index: 5,
        };
        assert_eq!(postback.encode(), "ex:2:5");
        assert_eq!(Postback::parse("ex:2:5"), Some(postback));
        assert_eq!(Postback::parse("  ex:2:5\n"), Some(postback));
        assert_eq!(postback.use_case(), "ex:5");
        assert_eq!(postback.position(), 2);
        assert_eq!(postback.source(), SuggestionSource::Static);
    }

    #[test]
    fn only_the_exact_shape_parses() {
        for text in [
            "",
            "ex:",
            "ex:1",
            "ex:1:",
            "ex::1",
            "ex:+1:2",
            "ex:1:-2",
            "ex:1: 2",
            "ex:1:2:3",
            "EX:1:2",
            "/plan",
            "What is my FTP?",
        ] {
            assert_eq!(Postback::parse(text), None, "{text:?} must not parse");
        }
    }

    #[test]
    fn a_command_button_is_not_a_postback() {
        assert!(!is_postback("/agent add @fuel"));
        assert!(is_postback("ex:0:0"));
    }
}
