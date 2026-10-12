// ABOUTME: Suggestion postbacks — the opaque value a tapped starter or next step sends, resolved before dispatch
// ABOUTME: Makes a tap measurable (suggestion.shown / .tapped, InputSource) and keeps the id out of the transcript
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
//! tagged [`InputSource::UseCase`], and
//! `suggestion.tapped` records which suggestion, from which slot.
//!
//! Three grammars:
//!
//! - `uc:<position>:<id>` — the catalogue starter `id`, shown in slot
//!   `position`. It resolves to its slash command, which then dispatches, or
//!   to its prompt in the athlete's language.
//! - `nx:<position>:<id>` — the catalogue entry `id` offered as a next step
//!   once a reply ended (carnet#830), in slot `position`. It resolves exactly
//!   as a starter does, but the turn is tagged
//!   [`InputSource::NextStep`] and reported under the `next_step` surface, so
//!   a step a reply handed over is never counted as a starter the athlete
//!   picked unprompted.
//! - `ex:<position>:<index>` — the agent's Example Input at `index` in
//!   `starter_samples` order, shown in slot `position`. The two numbers are
//!   kept apart because ranked starters can take the slots ahead of the
//!   examples.
//!
//! A value parses only when it is exactly that shape, so a question an athlete
//! types — and every slash-command button, whose value starts with `/` — is
//! never mistaken for one.
//!
//! The use-case starters themselves are ranked and worded in
//! [`pierre_services::use_case_rank`], which `/help` reaches too; an empty
//! thread offers them through [`offer_empty_thread_starters`].

use chrono::Utc;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_contremaitre::use_case_catalogue::{is_use_case_id, UseCaseCatalogue, UseCaseRun};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConversationRecord, InputSource, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_services::use_case_rank::{
    offer, rank_for_athlete, rotation_for, starter_category, use_case_line, use_case_postback,
    OfferedStarter, USE_CASE_POSTBACK_PREFIX,
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent_welcome::{localized_agent, starter_samples};
use crate::envelope::TurnAction;
use crate::surface_profile::SurfaceId;

/// The `surface` an agent welcome's starters are reported under.
pub(crate) const SURFACE_AGENT_WELCOME: &str = "agent_welcome";

/// The `surface` the next steps a reply ends on are reported under.
pub(crate) const SURFACE_NEXT_STEP: &str = "next_step";

/// The `surface` the starters of an empty thread are reported under: the web
/// pane with no thread open, the phone's thread before its first message.
pub(crate) const SURFACE_EMPTY_THREAD: &str = "empty_thread";

const EXAMPLE_PREFIX: &str = "ex:";
const NEXT_STEP_PREFIX: &str = "nx:";

/// Where a suggestion's words came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionSource {
    /// The agent's own authored Example Inputs, the same for every athlete.
    Static,
    /// A catalogue starter ranked from the athlete's state.
    State,
    /// A catalogue entry a reply offered as what to do next.
    NextStep,
}

impl SuggestionSource {
    /// The analytics value for this source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::State => "state",
            Self::NextStep => "next_step",
        }
    }
}

/// A suggestion's postback, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Postback {
    /// The bound agent's Example Input at `index`, in [`starter_samples`]
    /// order, offered in slot `position`.
    Example {
        /// 0-based slot the suggestion was shown in.
        position: usize,
        /// 0-based index into the agent's normalized Example Inputs.
        index: usize,
    },
    /// The catalogue starter `id`, offered in slot `position`.
    UseCase {
        /// 0-based slot the suggestion was shown in.
        position: usize,
        /// The starter's catalogue id.
        id: String,
    },
    /// The catalogue entry `id`, offered as a next step in slot `position`.
    NextStep {
        /// 0-based slot the step was shown in.
        position: usize,
        /// The entry's catalogue id.
        id: String,
    },
}

impl Postback {
    /// The value a control carries for this suggestion.
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            Self::Example { position, index } => format!("{EXAMPLE_PREFIX}{position}:{index}"),
            Self::UseCase { position, id } => use_case_postback(*position, id),
            Self::NextStep { position, id } => format!("{NEXT_STEP_PREFIX}{position}:{id}"),
        }
    }

    /// Parse a turn's text as a suggestion postback; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix(EXAMPLE_PREFIX) {
            let (position, index) = rest.split_once(':')?;
            return Some(Self::Example {
                position: parse_slot(position)?,
                index: parse_slot(index)?,
            });
        }
        if let Some(rest) = text.strip_prefix(NEXT_STEP_PREFIX) {
            let (position, id) = catalogue_slot(rest)?;
            return Some(Self::NextStep { position, id });
        }
        let (position, id) = catalogue_slot(text.strip_prefix(USE_CASE_POSTBACK_PREFIX)?)?;
        Some(Self::UseCase { position, id })
    }

    /// The suggestion's identity in analytics: a starter's catalogue id, or
    /// `ex:<index>` for an example — the same whichever slot it was offered in.
    #[must_use]
    pub fn use_case(&self) -> String {
        match self {
            Self::Example { index, .. } => format!("{EXAMPLE_PREFIX}{index}"),
            Self::UseCase { id, .. } | Self::NextStep { id, .. } => id.clone(),
        }
    }

    /// The slot the suggestion was shown in.
    #[must_use]
    pub const fn position(&self) -> usize {
        match self {
            Self::Example { position, .. }
            | Self::UseCase { position, .. }
            | Self::NextStep { position, .. } => *position,
        }
    }

    /// Where the suggestion's words came from.
    #[must_use]
    pub const fn source(&self) -> SuggestionSource {
        match self {
            Self::Example { .. } => SuggestionSource::Static,
            Self::UseCase { .. } => SuggestionSource::State,
            Self::NextStep { .. } => SuggestionSource::NextStep,
        }
    }

    /// The placement a tap on the suggestion is reported under: a welcome's
    /// starter, or a step a reply ended on. A tap carries nothing but its
    /// grammar, so a starter is a welcome's whichever surface showed it; a
    /// showing names its own placement ([`SuggestionEvent::surface`]).
    #[must_use]
    pub const fn surface(&self) -> &'static str {
        match self {
            Self::Example { .. } | Self::UseCase { .. } => SURFACE_AGENT_WELCOME,
            Self::NextStep { .. } => SURFACE_NEXT_STEP,
        }
    }

    /// How a turn this suggestion resolved into was produced.
    #[must_use]
    pub const fn input_source(&self) -> InputSource {
        match self {
            Self::Example { .. } | Self::UseCase { .. } => InputSource::UseCase,
            Self::NextStep { .. } => InputSource::NextStep,
        }
    }

    /// The catalogue entry the suggestion stands for; `None` for an example.
    #[must_use]
    pub fn catalogue_id(&self) -> Option<&str> {
        match self {
            Self::Example { .. } => None,
            Self::UseCase { id, .. } | Self::NextStep { id, .. } => Some(id),
        }
    }
}

/// Whether a control's `value` is a suggestion postback, whose text fallback
/// shows the label alone: the value means nothing typed back.
#[must_use]
pub fn is_postback(value: &str) -> bool {
    Postback::parse(value).is_some()
}

/// `<position>:<id>` after a catalogue prefix, with an id of the catalogue's
/// shape.
fn catalogue_slot(rest: &str) -> Option<(usize, String)> {
    let (position, id) = rest.split_once(':')?;
    if !is_use_case_id(id) {
        return None;
    }
    Some((parse_slot(position)?, id.to_owned()))
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
/// A starter resolves against the pinned catalogue: its command, or its
/// prompt read from `strings` in `tap.locale`. An example resolves against
/// the agent the thread is bound to, read in `tap.locale` and normalized
/// exactly as the welcome normalized it, so an index points at the same words
/// the athlete saw.
///
/// # Errors
///
/// Returns the repository error when the agent or its translation cannot be
/// read. A transient failure is not reported as a suggestion gone stale.
pub(crate) async fn resolve(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    tap: &TapContext<'_>,
    text: &str,
) -> AppResult<Resolution> {
    let Some(postback) = Postback::parse(text) else {
        return Ok(Resolution::NotAPostback);
    };
    let words = match &postback {
        Postback::Example { index, .. } => example_words(repos, tap, *index).await?,
        Postback::UseCase { id, .. } | Postback::NextStep { id, .. } => {
            use_case_words(UseCaseCatalogue::pinned(), strings, tap.locale, id)
        }
    };
    Ok(match words {
        Some(content) => Resolution::Resolved { content, postback },
        None => Resolution::Unresolvable { postback },
    })
}

/// The bound agent's Example Input at `index`, or `None` when the thread has
/// no agent, the agent is gone, or it has fewer examples now.
async fn example_words(
    repos: &RepositoryRegistry,
    tap: &TapContext<'_>,
    index: usize,
) -> AppResult<Option<String>> {
    let Some(agent_id) = tap.conversation.and_then(|c| c.agent_id.as_deref()) else {
        return Ok(None);
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
        return Ok(None);
    };
    Ok(starter_samples(&localized.sample_prompts)
        .into_iter()
        .nth(index))
}

/// What the catalogue entry `id` stands for: its slash command, or its prompt
/// in `locale`. `None` when the catalogue no longer lists it or its prompt has
/// no words in the strings.
fn use_case_words(
    catalogue: &UseCaseCatalogue,
    strings: &MessagingStringsRegistry,
    locale: &str,
    id: &str,
) -> Option<String> {
    let entry = catalogue.get(id)?;
    let words = match &entry.run {
        UseCaseRun::Command(command) => command.clone(),
        UseCaseRun::Prompt => strings.get(&entry.prompt_key(), locale),
    };
    (!words.trim().is_empty()).then_some(words)
}

/// The text form of a reply's untitled suggestion controls.
///
/// For a channel that lists them rather than drawing them: one `- ` line per
/// control, each written as [`use_case_line`] writes its catalogue entry.
///
/// `None` unless every control is a catalogue postback whose entry still has
/// words — a set it cannot write whole is left to the caller's own rendering,
/// so no control is dropped from it silently.
#[must_use]
pub fn suggestion_lines(
    actions: &[TurnAction],
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let catalogue = UseCaseCatalogue::pinned();
    let lines = actions
        .iter()
        .map(|action| {
            let postback = Postback::parse(&action.value)?;
            let entry = catalogue.get(postback.catalogue_id()?)?;
            use_case_line(entry, &action.label, strings, locale)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(
        lines
            .iter()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Who a suggestion event is about and where it happened.
pub(crate) struct SuggestionEvent<'a> {
    /// The athlete the suggestion was offered to.
    pub user_id: &'a str,
    /// Tenant of the conversation the suggestion lives in.
    pub tenant_id: TenantId,
    /// The placement: [`SURFACE_AGENT_WELCOME`], [`SURFACE_EMPTY_THREAD`] or
    /// [`SURFACE_NEXT_STEP`] where it was shown, [`Postback::surface`] for a
    /// tap.
    pub surface: &'a str,
    /// The chat surface, as [`crate::SurfaceId::as_str`] names it.
    pub channel: &'a str,
}

/// Record that `postback` was posted into a thread.
pub(crate) fn emit_shown(event: &SuggestionEvent<'_>, postback: &Postback) {
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
pub(crate) fn emit_tapped(event: &SuggestionEvent<'_>, postback: &Postback) {
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

/// Report the athlete's tap on `postback` and, for a catalogue entry — a
/// ranked starter or a next step — count it under `exposure_tenant`: a
/// `once` entry is used whether a welcome offered it or a reply did.
/// Best-effort: an unrecorded tap only lets the entry be offered once more
/// than it should.
pub(crate) async fn record_tap(
    repos: &RepositoryRegistry,
    event: &SuggestionEvent<'_>,
    postback: &Postback,
    exposure_tenant: TenantId,
    user: Uuid,
) {
    emit_tapped(event, postback);
    if let Some(id) = postback.catalogue_id() {
        if let Err(e) = repos
            .use_case_exposures
            .record_use_case_tapped(exposure_tenant, user, id, Utc::now())
            .await
        {
            warn!(error = %e, use_case = %id, "a use-case tap could not be recorded");
        }
    }
}

/// Who tapped a starter or a next step on a channel that routes commands
/// itself.
pub struct CommandTap {
    /// The athlete who tapped.
    pub user_id: Uuid,
    /// The athlete's own tenant, where their exposures are counted.
    pub user_tenant_id: TenantId,
    /// Tenant of the thread the tap arrived in, reported with the tap.
    pub conversation_tenant_id: TenantId,
    /// The chat surface the tap arrived on.
    pub surface: SurfaceId,
}

/// A tapped starter or next step that runs a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TappedCommand {
    /// The command the catalogue entry runs, as the athlete would type it.
    pub command: String,
    /// How the athlete produced it: [`InputSource::UseCase`] for a starter,
    /// [`InputSource::NextStep`] for a step a reply offered — so its reply
    /// teaches the command it stood for.
    pub input_source: InputSource,
}

/// The command a tapped catalogue entry runs, when `text` is the postback of
/// a starter or a next step that runs one — reported and counted as a tap.
///
/// For a channel that routes a typed command itself before the turn — a
/// messaging channel answers a command privately in a room and deletes its
/// echo — so a tapped `/pillars` takes the same road as a typed one rather
/// than the turn's. `None` for anything else, a prompt entry or an example
/// included: the turn resolves those, and reports their taps, as it always
/// has.
pub async fn resolve_command_tap(
    repos: &RepositoryRegistry,
    tap: &CommandTap,
    text: &str,
) -> Option<TappedCommand> {
    let postback = Postback::parse(text)?;
    let UseCaseRun::Command(command) = &UseCaseCatalogue::pinned()
        .get(postback.catalogue_id()?)?
        .run
    else {
        return None;
    };
    let user_id = tap.user_id.to_string();
    record_tap(
        repos,
        &SuggestionEvent {
            user_id: &user_id,
            tenant_id: tap.conversation_tenant_id,
            surface: postback.surface(),
            channel: tap.surface.as_str(),
        },
        &postback,
        tap.user_tenant_id,
        tap.user_id,
    )
    .await;
    Some(TappedCommand {
        command: command.clone(),
        input_source: postback.input_source(),
    })
}

/// Who an empty thread's starters are offered to, and where.
pub struct EmptyThreadOffer<'a> {
    /// The athlete's tenant.
    pub tenant_id: TenantId,
    /// The athlete.
    pub user_id: Uuid,
    /// Locale the starters are worded in.
    pub locale: &'a str,
    /// The app surface that shows them.
    pub surface: SurfaceId,
}

/// The starters an empty thread offers: ranked for the agent a new thread
/// would be bound to, worded in the athlete's language, at most
/// [`pierre_services::use_case_rank::MAX_STARTERS`].
///
/// Each one is reported as shown and counted toward retiring it, as a
/// welcome's are: the app asks for them when it draws the empty thread, so
/// one request is one showing. They rotate by the athlete rather than by a
/// thread, since no thread exists yet.
///
/// # Errors
///
/// Returns the repository error when the athlete's state cannot be read, and
/// refuses a caller outside Dravr's apps when the starters were chosen from
/// data that may reach the athlete's own apps only.
pub async fn offer_empty_thread_starters(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    offer_to: &EmptyThreadOffer<'_>,
) -> AppResult<Vec<OfferedStarter>> {
    let category = starter_category(repos, offer_to.tenant_id, offer_to.user_id, None).await;
    let user_id = offer_to.user_id.to_string();
    let ranked = rank_for_athlete(
        repos,
        offer_to.tenant_id,
        offer_to.user_id,
        category,
        rotation_for(&user_id),
    )
    .await?;
    // Which starters fit says something about what the athlete's data holds:
    // "My last workout" means activities were read. An interface that may not
    // carry that data is not told what it implies (carnet#769).
    if !ai_scope::admit_derived(ranked.policy) {
        return Err(AppError::unavailable_over_transport(
            "these starters are not available over this interface",
        ));
    }
    let offered = offer(&ranked.entries, strings, offer_to.locale);
    let shown = SuggestionEvent {
        user_id: &user_id,
        tenant_id: offer_to.tenant_id,
        surface: SURFACE_EMPTY_THREAD,
        channel: offer_to.surface.as_str(),
    };
    for (position, starter) in offered.iter().enumerate() {
        emit_shown(
            &shown,
            &Postback::UseCase {
                position,
                id: starter.id.clone(),
            },
        );
    }
    let ids: Vec<&str> = offered.iter().map(|starter| starter.id.as_str()).collect();
    if !ids.is_empty() {
        if let Err(e) = repos
            .use_case_exposures
            .record_use_cases_shown(offer_to.tenant_id, offer_to.user_id, &ids, Utc::now())
            .await
        {
            warn!(error = %e, user_id = %offer_to.user_id, "empty-thread starters shown could not be counted");
        }
    }
    Ok(offered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::ActionKind;

    const CATALOGUE: &str = "
- {id: about_me, run: command, command: /pillars, requires: [dossier_empty], stage: first_session, domains: [any], repeat: once}
- {id: season, run: command, command: /season, requires: [pillars_done, no_season], stage: first_week, domains: [training], repeat: once}
- {id: today, run: command, command: /plan today, requires: [plan_active], stage: any, domains: [training], repeat: recurring}
";

    fn catalogue() -> UseCaseCatalogue {
        UseCaseCatalogue::parse(CATALOGUE).expect("the fixture parses")
    }

    #[test]
    fn an_example_postback_round_trips() {
        let postback = Postback::Example {
            position: 2,
            index: 5,
        };
        assert_eq!(postback.encode(), "ex:2:5");
        assert_eq!(Postback::parse("ex:2:5"), Some(postback.clone()));
        assert_eq!(Postback::parse("  ex:2:5\n"), Some(postback.clone()));
        assert_eq!(postback.use_case(), "ex:5");
        assert_eq!(postback.position(), 2);
        assert_eq!(postback.source(), SuggestionSource::Static);
    }

    #[test]
    fn a_use_case_postback_round_trips() {
        let postback = Postback::UseCase {
            position: 1,
            id: "last_workout".to_owned(),
        };
        assert_eq!(postback.encode(), "uc:1:last_workout");
        assert_eq!(Postback::parse("uc:1:last_workout"), Some(postback.clone()));
        assert_eq!(postback.use_case(), "last_workout");
        assert_eq!(postback.position(), 1);
        assert_eq!(postback.source(), SuggestionSource::State);
    }

    #[test]
    fn a_next_step_postback_round_trips_and_is_counted_apart_from_a_starter() {
        let step = Postback::NextStep {
            position: 2,
            id: "fortnight".to_owned(),
        };
        assert_eq!(step.encode(), "nx:2:fortnight");
        assert_eq!(Postback::parse("nx:2:fortnight"), Some(step.clone()));
        assert_eq!(step.use_case(), "fortnight");
        assert_eq!(step.catalogue_id(), Some("fortnight"));
        assert_eq!(step.position(), 2);
        assert_eq!(step.source().as_str(), "next_step");
        assert_eq!(step.surface(), "next_step");
        assert_eq!(step.input_source(), InputSource::NextStep);

        let starter = Postback::parse("uc:2:fortnight").expect("a starter");
        assert_eq!(starter.source().as_str(), "state");
        assert_eq!(starter.surface(), "agent_welcome");
        assert_eq!(starter.input_source(), InputSource::UseCase);
        let example = Postback::parse("ex:0:1").expect("an example");
        assert_eq!(example.surface(), "agent_welcome");
        assert_eq!(example.input_source(), InputSource::UseCase);
        assert_eq!(example.catalogue_id(), None);
    }

    #[test]
    fn a_next_step_resolves_to_the_words_its_entry_stands_for() {
        let catalogue = catalogue();
        let strings = MessagingStringsRegistry::new();
        let id = |text: &str| {
            Postback::parse(text)
                .and_then(|postback| postback.catalogue_id().map(ToOwned::to_owned))
                .expect("a catalogue postback")
        };
        assert_eq!(
            use_case_words(&catalogue, &strings, "en", &id("nx:0:season")).as_deref(),
            Some("/season")
        );
        assert_eq!(
            use_case_words(&catalogue, &strings, "en", &id("nx:1:today")).as_deref(),
            Some("/plan today")
        );
    }

    #[test]
    fn untitled_steps_are_listed_as_the_command_to_type_or_the_words_to_send() {
        let strings = MessagingStringsRegistry::new();
        let pinned = UseCaseCatalogue::pinned();
        let season = pinned.get("season").expect("season is catalogued");
        let connect = pinned.get("connect").expect("connect is catalogued");
        let actions = [
            TurnAction {
                label: "Plan my season".to_owned(),
                kind: ActionKind::Postback,
                value: Postback::NextStep {
                    position: 0,
                    id: season.id.clone(),
                }
                .encode(),
            },
            TurnAction {
                label: "Connect my watch".to_owned(),
                kind: ActionKind::Postback,
                value: Postback::NextStep {
                    position: 1,
                    id: connect.id.clone(),
                }
                .encode(),
            },
        ];
        let prompt = strings.get(&connect.prompt_key(), "en");
        assert!(!prompt.is_empty(), "connect's prompt has words");
        assert_eq!(
            suggestion_lines(&actions, &strings, "en").as_deref(),
            Some(format!("- /season — Plan my season\n- {prompt}").as_str())
        );

        let command_button = TurnAction {
            label: "Plan".to_owned(),
            kind: ActionKind::Postback,
            value: "/plan".to_owned(),
        };
        assert_eq!(
            suggestion_lines(&[actions[0].clone(), command_button], &strings, "en"),
            None,
            "a set it cannot list whole is left to the card"
        );
        assert_eq!(suggestion_lines(&[], &strings, "en"), None);
    }

    #[test]
    fn a_use_case_resolves_to_its_command_or_its_prompt() {
        let catalogue = catalogue();
        let strings = MessagingStringsRegistry::new();
        assert_eq!(
            use_case_words(&catalogue, &strings, "en", "about_me").as_deref(),
            Some("/pillars")
        );
        assert_eq!(
            use_case_words(&catalogue, &strings, "en", "today").as_deref(),
            Some("/plan today")
        );
        assert_eq!(use_case_words(&catalogue, &strings, "en", "gone"), None);
    }

    #[test]
    fn the_pinned_starters_carry_their_prompts() {
        let strings = MessagingStringsRegistry::new();
        let pinned = UseCaseCatalogue::pinned();
        assert!(!pinned.entries().is_empty(), "the pinned catalogue loads");
        for entry in pinned.entries() {
            for locale in ["en", "fr"] {
                let words = use_case_words(pinned, &strings, locale, &entry.id)
                    .unwrap_or_else(|| panic!("{} has no words in {locale}", entry.id));
                assert!(!words.starts_with("uc:"));
            }
        }
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
            "uc:",
            "uc:1",
            "uc:1:",
            "uc::load",
            "uc:x:load",
            "uc:1:Load",
            "uc:1:load now",
            "uc:1:_load",
            "uc:1:/pillars",
            "nx:",
            "nx:1",
            "nx::season",
            "nx:-1:season",
            "nx:1:Season",
            "NX:1:season",
            "nx:1:season now",
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
        assert!(is_postback("uc:0:connect"));
        assert!(is_postback("nx:0:fortnight"));
    }
}
