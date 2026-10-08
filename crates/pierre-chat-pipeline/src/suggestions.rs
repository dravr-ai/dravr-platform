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
//! Two grammars:
//!
//! - `uc:<position>:<id>` — the catalogue starter `id`, shown in slot
//!   `position`. It resolves to its slash command, which then dispatches, or
//!   to its prompt in the athlete's language.
//! - `ex:<position>:<index>` — the agent's Example Input at `index` in
//!   `starter_samples` order, shown in slot `position`. The two numbers are
//!   kept apart because ranked starters can take the slots ahead of the
//!   examples.
//!
//! A value parses only when it is exactly that shape, so a question an athlete
//! types — and every slash-command button, whose value starts with `/` — is
//! never mistaken for one.
//!
//! [`rank`] picks the use-case starters a welcome offers from the athlete's
//! state, out of dravr-contremaitre's catalogue.

use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_contremaitre::use_case_catalogue::{
    is_use_case_id, Domains, Repeat, Stage, UseCase, UseCaseCatalogue, UseCaseRun,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{AgentCategory, ConversationRecord, TenantId};
use pierre_database::repositories::UseCaseExposure;
use pierre_database::RepositoryRegistry;
use pierre_services::athlete_state::AthleteState;
use tracing::info;
use uuid::Uuid;

use crate::agent_welcome::{localized_agent, starter_samples, MAX_STARTERS};

/// The `surface` an agent welcome's starters are reported under.
pub(crate) const SURFACE_AGENT_WELCOME: &str = "agent_welcome";

const EXAMPLE_PREFIX: &str = "ex:";
const USE_CASE_PREFIX: &str = "uc:";

/// Where a suggestion's words came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionSource {
    /// The agent's own authored Example Inputs, the same for every athlete.
    Static,
    /// A catalogue starter ranked from the athlete's state.
    State,
}

impl SuggestionSource {
    /// The analytics value for this source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::State => "state",
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
}

impl Postback {
    /// The value a control carries for this suggestion.
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            Self::Example { position, index } => format!("{EXAMPLE_PREFIX}{position}:{index}"),
            Self::UseCase { position, id } => format!("{USE_CASE_PREFIX}{position}:{id}"),
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
        let (position, id) = text.strip_prefix(USE_CASE_PREFIX)?.split_once(':')?;
        if !is_use_case_id(id) {
            return None;
        }
        Some(Self::UseCase {
            position: parse_slot(position)?,
            id: id.to_owned(),
        })
    }

    /// The suggestion's identity in analytics: a starter's catalogue id, or
    /// `ex:<index>` for an example — the same whichever slot it was offered in.
    #[must_use]
    pub fn use_case(&self) -> String {
        match self {
            Self::Example { index, .. } => format!("{EXAMPLE_PREFIX}{index}"),
            Self::UseCase { id, .. } => id.clone(),
        }
    }

    /// The slot the suggestion was shown in.
    #[must_use]
    pub const fn position(&self) -> usize {
        match self {
            Self::Example { position, .. } | Self::UseCase { position, .. } => *position,
        }
    }

    /// Where the suggestion's words came from.
    #[must_use]
    pub const fn source(&self) -> SuggestionSource {
        match self {
            Self::Example { .. } => SuggestionSource::Static,
            Self::UseCase { .. } => SuggestionSource::State,
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
        Postback::UseCase { id, .. } => {
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

/// What the starter `id` stands for: its slash command, or its prompt in
/// `locale`. `None` when the catalogue no longer lists it or its prompt has no
/// words in the strings.
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

/// Welcomes a starter may be offered in without a tap before the ranker stops
/// offering it (D10: "used" is a tap, not a showing).
const MAX_UNTAPPED_SHOWINGS: u32 = 3;

/// The use-case starters a welcome offers the athlete, at most
/// [`MAX_STARTERS`], for an agent of `category`.
///
/// A starter is offered when every requirement holds for `state`, unless the
/// athlete retired it: a `once` starter after its tap, any starter after
/// [`MAX_UNTAPPED_SHOWINGS`] welcomes without one. What remains is ordered by
/// stage — the athlete's own, then `any`, then the rest — keeping catalogue
/// order within each, so the catalogue's order is its priority. The first
/// slot may hold one cross-domain setup starter (D13); the rest are the
/// agent's own domain (D4). The chosen starters are then rotated by
/// `rotation`, so a tap says something about the starter rather than its slot.
#[must_use]
pub(crate) fn rank<'a>(
    catalogue: &'a UseCaseCatalogue,
    state: &AthleteState,
    category: AgentCategory,
    exposures: &[UseCaseExposure],
    rotation: usize,
) -> Vec<&'a UseCase> {
    let mut ordered: Vec<&UseCase> = catalogue
        .entries()
        .iter()
        .filter(|entry| entry.eligible(|predicate| state.holds(predicate)))
        .filter(|entry| !retired(entry, exposures))
        .collect();
    ordered.sort_by_key(|entry| stage_order(entry.stage, state.stage));

    let mut chosen: Vec<&UseCase> = ordered
        .iter()
        .copied()
        .find(|entry| entry.domains == Domains::Any)
        .into_iter()
        .collect();
    let room = MAX_STARTERS.saturating_sub(chosen.len());
    chosen.extend(
        ordered
            .iter()
            .copied()
            .filter(|entry| entry.domains.includes(category))
            .take(room),
    );
    if !chosen.is_empty() {
        let len = chosen.len();
        chosen.rotate_left(rotation % len);
    }
    chosen
}

/// A stable rotation for `conversation_id`: the same thread always shows its
/// starters in the same order, and threads spread over every order.
#[must_use]
pub(crate) fn rotation_for(conversation_id: &str) -> usize {
    conversation_id.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(usize::from(byte))
    })
}

/// Whether the athlete retired `entry`: tapped a `once` starter, or let any
/// starter go untapped through [`MAX_UNTAPPED_SHOWINGS`] welcomes.
fn retired(entry: &UseCase, exposures: &[UseCaseExposure]) -> bool {
    exposures
        .iter()
        .find(|exposure| exposure.use_case_id == entry.id)
        .is_some_and(|exposure| {
            (entry.repeat == Repeat::Once && exposure.tapped_at.is_some())
                || exposure.shown_count >= MAX_UNTAPPED_SHOWINGS
        })
}

/// Sort key: the athlete's own stage first, then `any`, then the others.
fn stage_order(entry: Stage, athlete: Stage) -> u8 {
    if entry == athlete {
        0
    } else if entry == Stage::Any {
        1
    } else {
        2
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pierre_services::athlete_state::{ActivityHistory, DossierCoverage};

    const CATALOGUE: &str = "
- {id: connect, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [any], repeat: once}
- {id: about_me, run: command, command: /pillars, requires: [dossier_empty], stage: first_session, domains: [any], repeat: once}
- {id: last_workout, run: prompt, command: null, requires: [has_activities], stage: first_session, domains: [analysis, training], repeat: recurring}
- {id: zone2, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [training], repeat: once}
- {id: fuel, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [nutrition], repeat: once}
- {id: season, run: command, command: /season, requires: [pillars_done, no_season], stage: first_week, domains: [training], repeat: once}
- {id: recovered, run: prompt, command: null, requires: [has_recovery_source | weeks_of_data>=2], stage: any, domains: [recovery, training], repeat: recurring}
- {id: today, run: command, command: /plan today, requires: [plan_active], stage: any, domains: [training], repeat: recurring}
";

    fn catalogue() -> UseCaseCatalogue {
        UseCaseCatalogue::parse(CATALOGUE).expect("the fixture parses")
    }

    /// A new athlete: nothing connected, nothing told, no plan.
    const fn newcomer() -> AthleteState {
        AthleteState {
            stage: Stage::FirstSession,
            has_provider: false,
            activities: ActivityHistory::Absent,
            weeks_of_data: 0,
            dossier: DossierCoverage {
                covered: 0,
                complete: false,
                empty: true,
            },
            season_set: false,
            plan_active: false,
            recovery_source: false,
            calendar_writable: false,
        }
    }

    /// A first-week athlete with a recovery band and three weeks of rides,
    /// pillars done, no season yet.
    const fn settled() -> AthleteState {
        AthleteState {
            stage: Stage::FirstWeek,
            has_provider: true,
            activities: ActivityHistory::Present,
            weeks_of_data: 3,
            dossier: DossierCoverage {
                covered: 6,
                complete: true,
                empty: false,
            },
            recovery_source: true,
            ..newcomer()
        }
    }

    fn ids(starters: &[&UseCase]) -> Vec<String> {
        starters.iter().map(|s| s.id.clone()).collect()
    }

    fn exposure(id: &str, shown_count: u32, tapped: bool) -> UseCaseExposure {
        UseCaseExposure {
            use_case_id: id.to_owned(),
            shown_count,
            last_shown_at: None,
            tapped_at: tapped.then(Utc::now),
        }
    }

    #[test]
    fn a_newcomer_gets_one_setup_starter_and_only_no_data_starters_on_topic() {
        let catalogue = catalogue();
        let rank_for = |category| ids(&rank(&catalogue, &newcomer(), category, &[], 0));
        assert_eq!(rank_for(AgentCategory::Training), ["connect", "zone2"]);
        assert_eq!(rank_for(AgentCategory::Nutrition), ["connect", "fuel"]);
        assert_eq!(rank_for(AgentCategory::Recovery), ["connect"]);
        assert_eq!(rank_for(AgentCategory::Custom), ["connect"]);
    }

    #[test]
    fn the_athletes_own_stage_leads_then_any_then_the_rest() {
        let catalogue = catalogue();
        let ranked = rank(&catalogue, &settled(), AgentCategory::Training, &[], 0);
        assert_eq!(ids(&ranked), ["season", "recovered", "last_workout"]);
    }

    #[test]
    fn a_tapped_setup_starter_gives_its_slot_to_the_next() {
        let catalogue = catalogue();
        let tapped = [exposure("connect", 0, true)];
        let ranked = rank(&catalogue, &newcomer(), AgentCategory::Training, &tapped, 0);
        assert_eq!(ids(&ranked), ["about_me", "zone2"]);
    }

    #[test]
    fn three_untapped_showings_retire_a_starter_and_a_tap_keeps_a_recurring_one() {
        let catalogue = catalogue();
        let ignored = [exposure("recovered", 3, false)];
        let ranked = rank(&catalogue, &settled(), AgentCategory::Training, &ignored, 0);
        assert_eq!(ids(&ranked), ["season", "last_workout"]);

        let used = [
            exposure("recovered", 0, true),
            exposure("last_workout", 2, false),
        ];
        let ranked = rank(&catalogue, &settled(), AgentCategory::Training, &used, 0);
        assert_eq!(ids(&ranked), ["season", "recovered", "last_workout"]);
    }

    #[test]
    fn the_rotation_moves_every_starter_and_is_stable_per_thread() {
        let catalogue = catalogue();
        let at = |rotation| {
            ids(&rank(
                &catalogue,
                &settled(),
                AgentCategory::Training,
                &[],
                rotation,
            ))
        };
        assert_eq!(at(1), ["recovered", "last_workout", "season"]);
        assert_eq!(at(3), at(0));
        assert_eq!(rotation_for("conv-1"), rotation_for("conv-1"));
        assert_ne!(rotation_for("conv-1") % 3, rotation_for("conv-2") % 3);
    }

    #[test]
    fn nothing_eligible_ranks_nothing() {
        let catalogue = catalogue();
        let quiet = AthleteState {
            has_provider: true,
            dossier: DossierCoverage {
                covered: 1,
                complete: false,
                empty: false,
            },
            ..newcomer()
        };
        assert!(rank(&catalogue, &quiet, AgentCategory::Nutrition, &[], 5).is_empty());
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
    }
}
