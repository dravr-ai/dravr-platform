// ABOUTME: Ranks the catalogue's use-case starters from the athlete's state and words them in their language
// ABOUTME: One ranker for the agent welcome, the empty thread's links and /help's buttons, so every surface offers the same
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Use-case starters, ranked (carnet#828, carnet#829).
//!
//! An athlete meets the same few starters wherever Dravr offers them: in an
//! agent's welcome, as the links of an empty thread, as `/help`'s buttons.
//! [`rank`] picks them from dravr-contremaitre's catalogue against the
//! athlete's [`AthleteState`], [`worded`] reads each one's label in the
//! athlete's language, and [`use_case_postback`] is what a tap sends back —
//! `uc:<slot>:<id>`, which the turn resolves into the command or the prompt
//! the starter stands for.
//!
//! It lives here rather than beside the turn that resolves a tap because
//! `/help` is a command handler, and the command crate sits below the chat
//! pipeline.

use chrono::Utc;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_contremaitre::use_case_catalogue::{
    Domains, Repeat, Stage, UseCase, UseCaseCatalogue, UseCaseRun,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{AgentCategory, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::UseCaseExposure;
use pierre_database::RepositoryRegistry;
use tracing::warn;
use uuid::Uuid;

use crate::athlete_state::AthleteState;
use crate::conversation_forge::selected_or_system_agent;
use crate::first_run::{done_flows, FirstRun};

/// The most starters a surface offers.
///
/// Three fit one glance and one row of chips on a phone, and three is the
/// smallest cap any channel puts on a card's buttons — `WhatsApp` refuses a
/// fourth reply button outright.
pub const MAX_STARTERS: usize = 3;

/// What a starter's postback opens with: `uc:<slot>:<id>`.
pub const USE_CASE_POSTBACK_PREFIX: &str = "uc:";

/// Showings a starter may have without a tap before the ranker stops
/// offering it (D10: "used" is a tap, not a showing).
const MAX_UNTAPPED_SHOWINGS: u32 = 3;

/// The postback a starter offered in slot `position` sends when tapped.
#[must_use]
pub fn use_case_postback(position: usize, id: &str) -> String {
    format!("{USE_CASE_POSTBACK_PREFIX}{position}:{id}")
}

/// The use-case starters offered to the athlete, at most [`MAX_STARTERS`],
/// for an agent of `category`.
///
/// A starter is offered when every requirement holds for `state`, unless the
/// athlete retired it: a `once` starter after its tap, any starter after
/// [`MAX_UNTAPPED_SHOWINGS`] showings without one. What remains is ordered by
/// stage — the athlete's own, then `any`, then the rest — keeping catalogue
/// order within each, so the catalogue's order is its priority. The first
/// slot may hold one cross-domain setup starter (D13); the rest are the
/// agent's own domain (D4). The chosen starters are then rotated by
/// `rotation`, so a tap says something about the starter rather than its slot.
///
/// The first run comes first (carnet#1099): while a first-run flow the
/// athlete can take is open, the lowest one takes the first slot whatever the
/// agent's domain, and stays there through the rotation; the setup starter
/// follows it, so an athlete with nothing connected is still told to connect. An open flow is
/// never retired by showings — it waits on a done rule, not on a tap — and a
/// `once` flow in `done` is retired like a tapped one.
///
/// No two starters offered together run the same thing — the same command,
/// or the same prompt: the higher-ranked one is kept and the other gives its
/// slot to the next.
#[must_use]
fn rank<'a>(
    catalogue: &'a UseCaseCatalogue,
    state: &AthleteState,
    category: AgentCategory,
    exposures: &[UseCaseExposure],
    done: &[String],
    rotation: usize,
) -> Vec<&'a UseCase> {
    let mut ordered: Vec<&UseCase> = catalogue
        .entries()
        .iter()
        .filter(|entry| entry.eligible(|predicate| state.holds(predicate)))
        .filter(|entry| !retired(entry, exposures, done))
        .collect();
    ordered.sort_by_key(|entry| stage_order(entry.stage, state.stage));

    let is_open_flow = |entry: &UseCase| entry.first_run.is_some() && !done.contains(&entry.id);
    let next_flow = ordered
        .iter()
        .copied()
        .filter(|entry| is_open_flow(entry))
        .min_by_key(|entry| entry.first_run);
    // Two starters that run the same thing are one offer: the higher-ranked
    // one keeps it, so the next flow wins over any starter sharing its run.
    let mut runs: Vec<RunTarget<'_>> = next_flow.iter().map(|flow| run_target(flow)).collect();
    // The setup starter is never another open flow: one flow is offered at
    // a time, and the next one takes the first slot when this one is done.
    let setup = ordered.iter().copied().find(|entry| {
        entry.domains == Domains::Any && !is_open_flow(entry) && !runs.contains(&run_target(entry))
    });
    runs.extend(setup.map(run_target));
    let taken = |entry: &UseCase| {
        [next_flow, setup]
            .iter()
            .flatten()
            .any(|t| t.id == entry.id)
    };
    let room = MAX_STARTERS
        .saturating_sub(usize::from(next_flow.is_some()) + usize::from(setup.is_some()));
    let mut chosen: Vec<&UseCase> = setup.into_iter().collect();
    let mut added = 0;
    for entry in ordered
        .iter()
        .copied()
        .filter(|entry| entry.domains.includes(category) && !taken(entry))
    {
        if added == room {
            break;
        }
        let run = run_target(entry);
        if !runs.contains(&run) {
            runs.push(run);
            chosen.push(entry);
            added += 1;
        }
    }
    rotate(&mut chosen, rotation);
    // The next flow keeps its slot; only the starters after it turn.
    if let Some(flow) = next_flow {
        chosen.insert(0, flow);
    }
    chosen
}

/// What a starter runs when tapped: its command, or its own prompt.
#[derive(Debug, PartialEq, Eq)]
enum RunTarget<'a> {
    /// The slash command, arguments included.
    Command(&'a str),
    /// The prompt the starter sends, named by its catalogue id.
    Prompt(&'a str),
}

/// The run target of `entry`, so two starters that would send the same
/// command are told apart from two that merely look alike.
fn run_target(entry: &UseCase) -> RunTarget<'_> {
    match &entry.run {
        UseCaseRun::Command(command) => RunTarget::Command(command.trim()),
        UseCaseRun::Prompt => RunTarget::Prompt(&entry.id),
    }
}

/// Turn `starters` left by `rotation`.
fn rotate(starters: &mut [&UseCase], rotation: usize) {
    if !starters.is_empty() {
        let len = starters.len();
        starters.rotate_left(rotation % len);
    }
}

/// A stable rotation for `key` — a thread id, or the athlete's id where no
/// thread exists yet: the same key always shows its starters in the same
/// order, and keys spread over every order.
#[must_use]
pub fn rotation_for(key: &str) -> usize {
    key.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(usize::from(byte))
    })
}

/// Whether the athlete retired `entry`: tapped a `once` starter, or let any
/// starter go untapped through [`MAX_UNTAPPED_SHOWINGS`] showings. An open
/// first-run flow is never retired by showings, only a `once` one by its
/// tap; a done `once` flow is retired whether or not it was tapped.
fn retired(entry: &UseCase, exposures: &[UseCaseExposure], done: &[String]) -> bool {
    let tapped = || {
        exposures
            .iter()
            .any(|exposure| exposure.use_case_id == entry.id && exposure.tapped_at.is_some())
    };
    if entry.first_run.is_some() {
        if !done.contains(&entry.id) {
            // A tap outside the thread that would make it done still uses a
            // once-only flow up; showings alone never do.
            return entry.repeat == Repeat::Once && tapped();
        }
        if entry.repeat == Repeat::Once {
            return true;
        }
    }
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

/// A catalogue starter in one locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordedStarter<'a> {
    /// The catalogue entry.
    pub use_case: &'a UseCase,
    /// What its button or link shows.
    pub label: String,
    /// How a surface that lists it as text writes it: a command starter as
    /// `/command — label`, a prompt starter as its prompt.
    pub line: String,
}

/// `entry` in `locale`, or `None` when the strings hold no words for it — a
/// starter with no label, or a prompt starter with no prompt, is not offered.
#[must_use]
pub fn worded<'a>(
    entry: &'a UseCase,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Option<WordedStarter<'a>> {
    let label = strings.get(&entry.label_key(), locale);
    if label.trim().is_empty() {
        return None;
    }
    let line = use_case_line(entry, &label, strings, locale)?;
    Some(WordedStarter {
        use_case: entry,
        label,
        line,
    })
}

/// The catalogue entry `entry` as a line of text, for a surface that lists
/// it rather than drawing a button labelled `label`.
///
/// A command entry is written `/command — label`, so the athlete can type it;
/// a prompt entry as its prompt in `locale`, the words a tap would have sent.
/// `None` when the strings hold no words for it. A welcome's starters and a
/// reply's next steps are listed this one way.
#[must_use]
pub fn use_case_line(
    entry: &UseCase,
    label: &str,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Option<String> {
    let line = match &entry.run {
        UseCaseRun::Command(command) => format!("{command} — {label}"),
        UseCaseRun::Prompt => strings.get(&entry.prompt_key(), locale),
    };
    (!line.trim().is_empty()).then_some(line)
}

/// A starter as a surface offers it: what it shows and what a tap sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedStarter {
    /// The catalogue id.
    pub id: String,
    /// What its button or link shows, in the athlete's language.
    pub label: String,
    /// What a tap sends: `uc:<slot>:<id>`.
    pub postback: String,
}

/// The ranked `entries` worded in `locale`, slot by slot: an entry with no
/// words is dropped and the next one takes its slot, so a postback's slot is
/// the one it was shown in.
#[must_use]
pub fn offer(
    entries: &[&UseCase],
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> Vec<OfferedStarter> {
    entries
        .iter()
        .filter_map(|entry| worded(entry, strings, locale))
        .enumerate()
        .map(|(position, starter)| OfferedStarter {
            id: starter.use_case.id.clone(),
            postback: use_case_postback(position, &starter.use_case.id),
            label: starter.label,
        })
        .collect()
}

/// What [`rank_for_athlete`] read and ranked.
#[derive(Debug, Clone)]
pub struct RankedStarters {
    /// The athlete's state the starters were ranked from.
    pub state: AthleteState,
    /// The policy anything derived from that state is stamped with.
    pub policy: TransportPolicy,
    /// The ranked starters, at most [`MAX_STARTERS`].
    pub entries: Vec<&'static UseCase>,
}

/// Read `user`'s state and exposures under `tenant`, and rank the pinned
/// catalogue for an agent of `category`.
///
/// Exposures that cannot be read rank as if none were shown: an athlete is
/// offered a starter once more than they should, never none.
///
/// # Errors
///
/// Propagates the error of the state read: ranking on a partial state could
/// offer a starter the athlete cannot use.
pub async fn rank_for_athlete(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: Uuid,
    category: AgentCategory,
    rotation: usize,
) -> AppResult<RankedStarters> {
    let (state, policy) = AthleteState::read(repos, tenant, user, Utc::now()).await?;
    let exposures = repos
        .use_case_exposures
        .use_case_exposures(tenant, user)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, user_id = %user, "starter exposures unreadable; ranking as if none were shown");
            Vec::new()
        });
    let catalogue = UseCaseCatalogue::pinned();
    // An unreadable first run ranks every flow open: one offered once more
    // than it should, never one hidden.
    let first_run = FirstRun::read(repos, tenant, user)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, user_id = %user, "first-run progress unreadable; ranking every flow open");
            FirstRun::default()
        });
    let done = done_flows(catalogue, &first_run);
    Ok(RankedStarters {
        state,
        policy,
        entries: rank(catalogue, &state, category, &exposures, &done, rotation),
    })
}

/// The category of the agent `user` talks to: `agent_id` when a thread is
/// bound to one, else the agent a new personal thread would be bound to —
/// their selection, else their own thread's default.
///
/// [`AgentCategory::Custom`] when no agent resolves, which ranks the
/// cross-domain setup starter alone: with no agent, no domain is the
/// athlete's.
pub async fn starter_category(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: Uuid,
    agent_id: Option<&str>,
) -> AgentCategory {
    let agent_id = match agent_id {
        Some(bound) => Some(bound.to_owned()),
        None => selected_or_system_agent(repos, tenant, user).await,
    };
    let Some(agent_id) = agent_id else {
        return AgentCategory::Custom;
    };
    match repos.agents.get_by_id(&agent_id, user, tenant).await {
        Ok(Some(agent)) => agent.category,
        Ok(None) => AgentCategory::Custom,
        Err(e) => {
            warn!(error = %e, agent_id, "starter agent unreadable; ranking the setup starter alone");
            AgentCategory::Custom
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::athlete_state::{ActivityHistory, DossierCoverage};

    const CATALOGUE: &str = "
- {id: connect, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [any], repeat: once}
- {id: about_me_more, run: command, command: /pillars, requires: [dossier_empty | pillars_open], stage: any, domains: [any], repeat: recurring}
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
            check_in_on: false,
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

    /// Strings whose English labels and prompts name their entry, except for
    /// a blank label on `zone2` and a blank prompt on `fuel`.
    fn strings() -> MessagingStringsRegistry {
        let strings = MessagingStringsRegistry::new();
        for entry in catalogue().entries() {
            let label = if entry.id == "zone2" {
                String::new()
            } else {
                format!("Label {}", entry.id)
            };
            strings.update(&entry.label_key(), "en", label, String::new());
            if entry.run == UseCaseRun::Prompt {
                let prompt = if entry.id == "fuel" {
                    String::new()
                } else {
                    format!("Prompt {}", entry.id)
                };
                strings.update(&entry.prompt_key(), "en", prompt, String::new());
            }
        }
        strings
    }

    #[test]
    fn a_newcomer_gets_one_setup_starter_and_only_no_data_starters_on_topic() {
        let catalogue = catalogue();
        let rank_for = |category| ids(&rank(&catalogue, &newcomer(), category, &[], &[], 0));
        assert_eq!(rank_for(AgentCategory::Training), ["connect", "zone2"]);
        assert_eq!(rank_for(AgentCategory::Nutrition), ["connect", "fuel"]);
        assert_eq!(rank_for(AgentCategory::Recovery), ["connect"]);
        assert_eq!(rank_for(AgentCategory::Custom), ["connect"]);
    }

    #[test]
    fn the_athletes_own_stage_leads_then_any_then_the_rest() {
        let catalogue = catalogue();
        let ranked = rank(&catalogue, &settled(), AgentCategory::Training, &[], &[], 0);
        assert_eq!(ids(&ranked), ["season", "recovered", "last_workout"]);
    }

    #[test]
    fn a_tapped_setup_starter_gives_its_slot_to_the_next() {
        let catalogue = catalogue();
        let tapped = [exposure("connect", 0, true)];
        let ranked = rank(
            &catalogue,
            &newcomer(),
            AgentCategory::Training,
            &tapped,
            &[],
            0,
        );
        assert_eq!(ids(&ranked), ["about_me_more", "zone2"]);
    }

    #[test]
    fn three_untapped_showings_retire_a_starter_and_a_tap_keeps_a_recurring_one() {
        let catalogue = catalogue();
        let ignored = [exposure("recovered", 3, false)];
        let ranked = rank(
            &catalogue,
            &settled(),
            AgentCategory::Training,
            &ignored,
            &[],
            0,
        );
        assert_eq!(ids(&ranked), ["season", "last_workout"]);

        let used = [
            exposure("recovered", 0, true),
            exposure("last_workout", 2, false),
        ];
        let ranked = rank(
            &catalogue,
            &settled(),
            AgentCategory::Training,
            &used,
            &[],
            0,
        );
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
                &[],
                rotation,
            ))
        };
        assert_eq!(at(1), ["recovered", "last_workout", "season"]);
        assert_eq!(at(3), at(0));
        assert_eq!(rotation_for("conv-1"), rotation_for("conv-1"));
        assert_ne!(rotation_for("conv-1") % 3, rotation_for("conv-2") % 3);
    }

    /// The four first-run flows beside two ordinary starters.
    const FLOWS: &str = "
- {id: meet_team, run: prompt, command: null, requires: [], stage: first_session, domains: [any], repeat: once, first_run: 1}
- {id: about_me_more, run: command, command: /pillars, requires: [dossier_empty | pillars_open], stage: any, domains: [any], repeat: recurring, first_run: 3}
- {id: last_workout, run: prompt, command: null, requires: [has_activities], stage: first_session, domains: [analysis, training], repeat: recurring, first_run: 2}
- {id: fuel, run: prompt, command: null, requires: [], stage: first_session, domains: [nutrition], repeat: once}
- {id: hydrate, run: prompt, command: null, requires: [], stage: first_session, domains: [nutrition], repeat: recurring}
- {id: check_in, run: prompt, command: null, requires: [check_in_on], stage: any, domains: [any], repeat: recurring, first_run: 4}
";

    fn catalogue_with_connect() -> UseCaseCatalogue {
        let connect = "- {id: connect, run: prompt, command: null, requires: [no_provider], stage: first_session, domains: [any], repeat: once}";
        UseCaseCatalogue::parse(&format!("{connect}{FLOWS}")).expect("the fixture parses")
    }

    fn flows_state() -> AthleteState {
        AthleteState {
            has_provider: true,
            activities: ActivityHistory::Present,
            dossier: DossierCoverage {
                covered: 2,
                complete: false,
                empty: false,
            },
            check_in_on: true,
            ..newcomer()
        }
    }

    fn done(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn the_next_open_flow_takes_the_first_slot_whatever_the_agents_domain() {
        let catalogue = UseCaseCatalogue::parse(FLOWS).expect("the fixture parses");
        let state = flows_state();
        let at = |finished: &[&str], rotation| {
            ids(&rank(
                &catalogue,
                &state,
                AgentCategory::Nutrition,
                &[],
                &done(finished),
                rotation,
            ))
        };
        assert_eq!(at(&[], 0), ["meet_team", "fuel", "hydrate"]);
        assert!(
            !at(&[], 0).contains(&"about_me_more".to_owned()),
            "one open flow at a time"
        );
        assert_eq!(
            at(&[], 1),
            ["meet_team", "hydrate", "fuel"],
            "the flow keeps its slot through the rotation"
        );
        assert_eq!(
            at(&["meet_team"], 0),
            ["last_workout", "fuel", "hydrate"],
            "a nutrition lead still offers the analysis flow"
        );
        assert_eq!(at(&["meet_team", "last_workout"], 0)[0], "about_me_more");
        assert_eq!(
            at(&["meet_team", "last_workout", "about_me_more"], 0)[0],
            "check_in"
        );
    }

    #[test]
    fn an_unconnected_athlete_is_offered_the_flow_then_the_setup_starter() {
        let catalogue = catalogue_with_connect();
        let state = AthleteState {
            has_provider: false,
            activities: ActivityHistory::Absent,
            ..flows_state()
        };
        let at = |rotation| {
            ids(&rank(
                &catalogue,
                &state,
                AgentCategory::Nutrition,
                &[],
                &[],
                rotation,
            ))
        };
        assert_eq!(at(0), ["meet_team", "connect", "fuel"]);
        assert_eq!(
            at(1),
            ["meet_team", "fuel", "connect"],
            "only the starters after the flow turn"
        );
    }

    #[test]
    fn a_flow_the_athlete_cannot_take_yet_gives_the_slot_to_the_next_one() {
        let catalogue = UseCaseCatalogue::parse(FLOWS).expect("the fixture parses");
        let state = AthleteState {
            activities: ActivityHistory::Absent,
            ..flows_state()
        };
        let ranked = rank(
            &catalogue,
            &state,
            AgentCategory::Training,
            &[],
            &done(&["meet_team"]),
            0,
        );
        assert_eq!(ids(&ranked), ["about_me_more"]);
    }

    #[test]
    fn showings_never_retire_an_open_flow_and_a_done_once_flow_is_retired() {
        let catalogue = UseCaseCatalogue::parse(FLOWS).expect("the fixture parses");
        let ignored = [exposure("meet_team", 5, false)];
        let ranked = rank(
            &catalogue,
            &flows_state(),
            AgentCategory::Nutrition,
            &ignored,
            &[],
            0,
        );
        assert_eq!(ids(&ranked)[0], "meet_team");
        let tapped = [exposure("meet_team", 0, true)];
        let ranked = rank(
            &catalogue,
            &flows_state(),
            AgentCategory::Nutrition,
            &tapped,
            &[],
            0,
        );
        assert!(
            !ids(&ranked).contains(&"meet_team".to_owned()),
            "a tapped once-only flow is used up even before it is done"
        );

        let everything_done = done(&["meet_team", "last_workout", "about_me_more", "check_in"]);
        let ranked = rank(
            &catalogue,
            &flows_state(),
            AgentCategory::Nutrition,
            &[],
            &everything_done,
            0,
        );
        assert!(
            !ids(&ranked).contains(&"meet_team".to_owned()),
            "a done once-only flow is not offered again"
        );
        assert_eq!(
            ids(&ranked)[0],
            "about_me_more",
            "a done recurring flow is an ordinary setup starter again"
        );
    }

    #[test]
    fn two_starters_that_run_the_same_command_are_offered_once() {
        let about_me = "- {id: about_me, run: command, command: /pillars, requires: [dossier_empty], stage: first_session, domains: [any], repeat: once}";
        let catalogue =
            UseCaseCatalogue::parse(&format!("{about_me}{FLOWS}")).expect("the fixture parses");
        let state = AthleteState {
            dossier: DossierCoverage {
                covered: 0,
                complete: false,
                empty: true,
            },
            ..flows_state()
        };
        assert!(catalogue
            .get("about_me")
            .is_some_and(|entry| entry.eligible(|predicate| state.holds(predicate))));
        assert!(catalogue
            .get("about_me_more")
            .is_some_and(|entry| entry.eligible(|predicate| state.holds(predicate))));
        let at = |finished: &[&str]| {
            ids(&rank(
                &catalogue,
                &state,
                AgentCategory::Nutrition,
                &[],
                &done(finished),
                0,
            ))
        };
        assert_eq!(
            at(&["meet_team", "last_workout"]),
            ["about_me_more", "fuel", "hydrate"],
            "the open flow keeps /pillars and about_me gives its setup slot away"
        );
        assert_eq!(
            at(&["meet_team"]),
            ["last_workout", "about_me", "fuel"],
            "with the flow not yet next, the setup starter runs /pillars alone"
        );
        let all_done = done(&["meet_team", "last_workout", "about_me_more", "check_in"]);
        let ranked = ids(&rank(
            &catalogue,
            &state,
            AgentCategory::Nutrition,
            &[],
            &all_done,
            0,
        ));
        assert_eq!(
            ranked
                .iter()
                .filter(|id| id.starts_with("about_me"))
                .count(),
            1,
            "{ranked:?}"
        );
    }

    #[test]
    fn nothing_eligible_ranks_nothing() {
        let catalogue = catalogue();
        let quiet = AthleteState {
            has_provider: true,
            dossier: DossierCoverage {
                covered: 6,
                complete: true,
                empty: false,
            },
            ..newcomer()
        };
        let got = rank(&catalogue, &quiet, AgentCategory::Nutrition, &[], &[], 5);
        assert!(got.is_empty(), "expected empty, got {got:?}");
    }

    #[test]
    fn a_command_starter_is_listed_with_its_command_and_a_prompt_starter_as_its_prompt() {
        let catalogue = catalogue();
        let strings = strings();
        let about_me_more = catalogue.get("about_me_more").expect("in the fixture");
        let worded_profile = worded(about_me_more, &strings, "en").expect("worded");
        assert_eq!(worded_profile.label, "Label about_me_more");
        assert_eq!(worded_profile.line, "/pillars — Label about_me_more");

        let connect = catalogue.get("connect").expect("in the fixture");
        assert_eq!(
            worded(connect, &strings, "en").map(|s| s.line).as_deref(),
            Some("Prompt connect")
        );
    }

    #[test]
    fn a_starter_without_a_label_or_a_prompt_is_not_worded() {
        let catalogue = catalogue();
        let strings = strings();
        let unlabelled = catalogue.get("zone2").expect("in the fixture");
        assert_eq!(worded(unlabelled, &strings, "en"), None);
        let unprompted = catalogue.get("fuel").expect("in the fixture");
        assert_eq!(worded(unprompted, &strings, "en"), None);
    }

    #[test]
    fn an_offer_numbers_its_slots_after_dropping_what_has_no_words() {
        let catalogue = catalogue();
        let strings = strings();
        let entries: Vec<&UseCase> = ["zone2", "connect", "about_me_more"]
            .iter()
            .map(|id| catalogue.get(id).expect("in the fixture"))
            .collect();
        let offered = offer(&entries, &strings, "en");
        assert_eq!(
            offered,
            [
                OfferedStarter {
                    id: "connect".to_owned(),
                    label: "Label connect".to_owned(),
                    postback: "uc:0:connect".to_owned(),
                },
                OfferedStarter {
                    id: "about_me_more".to_owned(),
                    label: "Label about_me_more".to_owned(),
                    postback: "uc:1:about_me_more".to_owned(),
                },
            ]
        );
    }
}
