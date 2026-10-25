// ABOUTME: The pillars walk's quiet offer to add the agent a newly covered pillar names to the athlete's team
// ABOUTME: Which agent is offered, the two postbacks it posts, and the one-line answer to each tap (carnet#1097)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The add-to-team offer (carnet#1097).
//!
//! When "Finish your profile" — the pillars walk — ends having covered a
//! pillar whose agent ([`agent_for_pillar`]) is not on the athlete's team,
//! its wrap-up offers that agent with two quiet links: "Add X to my team" and
//! "Not now". One agent per wrap-up, the first covered pillar in
//! [`Pillar::ALL`] order that qualifies.
//!
//! An agent qualifies while it is published in the Store, is not
//! coach-facing, is not RED-S, is not the athlete's lead, and is not already
//! on their team. A pillar is offered at most once: the offer is recorded as
//! shown in the athlete's starter exposures under [`exposure_id`], a "Not
//! now" is recorded there as its tap, and either row keeps that pillar from
//! being offered again. The id is no catalogue id, so the starter ranker and
//! the next-step picker, which look exposures up by catalogue id, never read
//! it.
//!
//! Each link posts `ta:<choice>:<pillar>` — the pillar, never an agent id,
//! so a value typed back names nothing the server does not resolve itself.
//! A tap is answered by the platform in one line, with no model call: "Add"
//! re-checks every rule above and joins the team through the one Store
//! install path ([`join_team`]); "Not now" records the dismissal.

use std::slice;

use chrono::Utc;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_AGENTS_TEAM_OFFER_ADDED, KEY_AGENTS_TEAM_OFFER_DISMISSED,
    KEY_USE_CASES_UNAVAILABLE,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{Agent, AgentHandle, Pillar, TenantId};
use pierre_database::views::AgentRepos;
use pierre_database::RepositoryRegistry;
use pierre_services::agent_store::join_team;
use pierre_services::agents::{coach_tools_for, require_bindable_agent};
use pierre_services::conversation_forge::selected_or_system_agent;
use pierre_services::team_rules::{agent_for_pillar, RED_S_AGENT_HANDLE};
use tracing::warn;
use uuid::Uuid;

/// What a team offer's postback opens with: `ta:<choice>:<pillar>`.
pub(crate) const TEAM_OFFER_PREFIX: &str = "ta:";

/// What an offered pillar's exposure id opens with: `team_offer_<pillar>`.
const EXPOSURE_PREFIX: &str = "team_offer_";

/// Which of the offer's two links was tapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamOfferChoice {
    /// "Add X to my team".
    Add,
    /// "Not now".
    NotNow,
}

impl TeamOfferChoice {
    /// The choice as its postback writes it.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::NotNow => "no",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "add" => Some(Self::Add),
            "no" => Some(Self::NotNow),
            _ => None,
        }
    }

    /// The link's place within the offer: "Add" first, "Not now" second.
    #[must_use]
    pub(crate) const fn position(self) -> usize {
        match self {
            Self::Add => 0,
            Self::NotNow => 1,
        }
    }
}

/// The postback the `choice` link of an offer on `pillar` sends.
#[must_use]
pub(crate) fn encode(choice: TeamOfferChoice, pillar: Pillar) -> String {
    format!("{TEAM_OFFER_PREFIX}{}:{}", choice.as_str(), pillar.as_str())
}

/// `<choice>:<pillar>` after [`TEAM_OFFER_PREFIX`]; `None` for anything else.
#[must_use]
pub(crate) fn parse(rest: &str) -> Option<(TeamOfferChoice, Pillar)> {
    let (choice, pillar) = rest.split_once(':')?;
    Some((TeamOfferChoice::parse(choice)?, Pillar::parse(pillar)?))
}

/// The starter-exposure id an offer on `pillar` is recorded under, and the
/// suggestion id analytics reports it as.
#[must_use]
pub(crate) fn exposure_id(pillar: Pillar) -> String {
    format!("{EXPOSURE_PREFIX}{}", pillar.as_str())
}

/// Who an offer is for, and the thread it is made in.
pub(crate) struct OfferFor<'a> {
    /// The athlete.
    pub user_id: Uuid,
    /// The athlete's own tenant, where their team and exposures live.
    pub tenant_id: TenantId,
    /// The agent the thread is bound to, which leads it.
    pub thread_agent: Option<&'a str>,
    /// Locale the agent's name is read in.
    pub locale: &'a str,
}

/// An agent the wrap-up offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfferedAgent {
    /// The newly covered pillar the agent answers.
    pub pillar: Pillar,
    /// The agent's name in the athlete's language.
    pub name: String,
}

/// The agent `pillar` offers `offer_for`, while it may join their team:
/// published, not coach-facing, not RED-S, and not their lead. Whether they
/// already hold it is the caller's question.
///
/// # Errors
///
/// Returns the repository error when the Store or the selection cannot be
/// read.
async fn offerable_agent(
    repos: &RepositoryRegistry,
    offer_for: &OfferFor<'_>,
    pillar: Pillar,
) -> AppResult<Option<Agent>> {
    let Some(handle) = agent_for_pillar(pillar).filter(|handle| *handle != RED_S_AGENT_HANDLE)
    else {
        return Ok(None);
    };
    let handle = AgentHandle::parse(handle)?;
    let Some(published) = repos
        .store_listings
        .find_published_by_handle(&handle)
        .await?
    else {
        return Ok(None);
    };
    let agent = published.agent;
    if agent.is_coach_facing() {
        return Ok(None);
    }
    let id = agent.id.to_string();
    let lead = selected_or_system_agent(repos, offer_for.tenant_id, offer_for.user_id).await;
    if offer_for.thread_agent == Some(id.as_str()) || lead.as_deref() == Some(id.as_str()) {
        return Ok(None);
    }
    Ok(Some(agent))
}

/// `agent`'s name in `locale`, or its canonical name when no translation
/// can be read.
async fn localized_name(repos: &RepositoryRegistry, mut agent: Agent, locale: &str) -> String {
    if let Err(e) = repos
        .agents
        .translate_agents(slice::from_mut(&mut agent), locale)
        .await
    {
        warn!(error = %e, agent_id = %agent.id, "offered agent's translation unreadable; offering its canonical name");
    }
    agent.title
}

/// The agent the wrap-up of a walk that covered `covered` offers, if any.
///
/// Best-effort: an athlete whose exposures or Store cannot be read is
/// offered nothing, so a pillar is never offered twice on a guess.
pub(crate) async fn pick(
    repos: &RepositoryRegistry,
    offer_for: &OfferFor<'_>,
    covered: &[Pillar],
) -> Option<OfferedAgent> {
    let exposures = match repos
        .use_case_exposures
        .use_case_exposures(offer_for.tenant_id, offer_for.user_id)
        .await
    {
        Ok(exposures) => exposures,
        Err(e) => {
            warn!(error = %e, user_id = %offer_for.user_id, "starter exposures unreadable; no agent is offered");
            return None;
        }
    };
    for pillar in Pillar::ALL.into_iter().filter(|p| covered.contains(p)) {
        let id = exposure_id(pillar);
        if exposures.iter().any(|exposure| exposure.use_case_id == id) {
            continue;
        }
        match agent_to_offer(repos, offer_for, pillar).await {
            Ok(Some(agent)) => {
                return Some(OfferedAgent {
                    pillar,
                    name: localized_name(repos, agent, offer_for.locale).await,
                });
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, pillar = %pillar, "offered agent or team unreadable; no agent is offered");
                return None;
            }
        }
    }
    None
}

/// The agent `pillar` offers, while it may join and the athlete does not
/// hold it yet.
///
/// # Errors
///
/// Returns the repository error when the Store, the selection or the team
/// cannot be read.
async fn agent_to_offer(
    repos: &RepositoryRegistry,
    offer_for: &OfferFor<'_>,
    pillar: Pillar,
) -> AppResult<Option<Agent>> {
    let Some(agent) = offerable_agent(repos, offer_for, pillar).await? else {
        return Ok(None);
    };
    let held = repos
        .store_listings
        .holds_install(
            &agent.id.to_string(),
            offer_for.user_id,
            offer_for.tenant_id,
        )
        .await?;
    Ok((!held).then_some(agent))
}

/// Record that the offer on `pillar` was shown, so it is not made again.
pub(crate) async fn record_shown(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    pillar: Pillar,
) {
    let id = exposure_id(pillar);
    if let Err(e) = repos
        .use_case_exposures
        .record_use_cases_shown(tenant_id, user_id, &[id.as_str()], Utc::now())
        .await
    {
        warn!(error = %e, pillar = %pillar, "team offer shown could not be recorded; it may be offered again");
    }
}

/// Record the athlete's tap on the offer on `pillar`.
async fn record_tapped(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    pillar: Pillar,
) {
    if let Err(e) = repos
        .use_case_exposures
        .record_use_case_tapped(tenant_id, user_id, &exposure_id(pillar), Utc::now())
        .await
    {
        warn!(error = %e, pillar = %pillar, "team offer tap could not be recorded; it may be offered again");
    }
}

/// The one line answering a tap on the `choice` link of the offer on
/// `pillar`.
///
/// "Add" joins the agent when it still qualifies — idempotently, so a second
/// tap confirms again — and answers that the agent may not join otherwise.
/// "Not now" records the dismissal.
pub(crate) async fn answer(
    repos: &RepositoryRegistry,
    strings: &MessagingStringsRegistry,
    offer_for: &OfferFor<'_>,
    choice: TeamOfferChoice,
    pillar: Pillar,
) -> String {
    match choice {
        TeamOfferChoice::NotNow => {
            record_tapped(repos, offer_for.tenant_id, offer_for.user_id, pillar).await;
            strings.get(KEY_AGENTS_TEAM_OFFER_DISMISSED, offer_for.locale)
        }
        TeamOfferChoice::Add => match join_offered(repos, offer_for, pillar).await {
            Ok(Some(name)) => {
                record_tapped(repos, offer_for.tenant_id, offer_for.user_id, pillar).await;
                strings.render(KEY_AGENTS_TEAM_OFFER_ADDED, offer_for.locale, &[&name])
            }
            Ok(None) => strings.get(KEY_USE_CASES_UNAVAILABLE, offer_for.locale),
            Err(e) => {
                warn!(error = %e, pillar = %pillar, "offered agent could not join the team");
                strings.get(KEY_USE_CASES_UNAVAILABLE, offer_for.locale)
            }
        },
    }
}

/// Put the agent the offer on `pillar` names on the athlete's team, and
/// return its name; `None` when it no longer qualifies.
///
/// # Errors
///
/// Returns the repository error of a read, or the install's.
async fn join_offered(
    repos: &RepositoryRegistry,
    offer_for: &OfferFor<'_>,
    pillar: Pillar,
) -> AppResult<Option<String>> {
    let Some(agent) = offerable_agent(repos, offer_for, pillar).await? else {
        return Ok(None);
    };
    let id = agent.id.to_string();
    require_bindable_agent(
        repos.agents.as_ref(),
        repos.users.as_ref(),
        &id,
        offer_for.user_id,
        offer_for.tenant_id,
    )
    .await?;
    let coach_tools = coach_tools_for(repos.users.as_ref(), offer_for.user_id).await;
    join_team(
        &AgentRepos::from_registry(repos),
        &id,
        offer_for.user_id,
        offer_for.tenant_id,
        coach_tools,
    )
    .await?;
    Ok(Some(localized_name(repos, agent, offer_for.locale).await))
}

#[cfg(test)]
mod tests {
    use pierre_contremaitre::use_case_catalogue::UseCaseCatalogue;

    use super::*;

    #[test]
    fn a_team_offer_postback_round_trips_and_fits_a_telegram_callback() {
        for pillar in Pillar::ALL {
            for choice in [TeamOfferChoice::Add, TeamOfferChoice::NotNow] {
                let value = encode(choice, pillar);
                // Telegram's callback_data carries at most 64 bytes.
                assert!(value.len() <= 64, "{value}");
                let rest = value.strip_prefix(TEAM_OFFER_PREFIX).expect("prefixed");
                assert_eq!(parse(rest), Some((choice, pillar)));
            }
        }
        assert_eq!(
            encode(TeamOfferChoice::Add, Pillar::Fuelling),
            "ta:add:fuelling"
        );
        assert_eq!(
            encode(TeamOfferChoice::NotNow, Pillar::SleepAndRecovery),
            "ta:no:sleep_and_recovery"
        );
    }

    #[test]
    fn anything_but_a_choice_and_a_known_pillar_is_no_team_offer() {
        for rest in [
            "add",
            "add:",
            "yes:fuelling",
            "add:nutrition",
            "add:fuelling:x",
            "ADD:fuelling",
            ":fuelling",
        ] {
            assert_eq!(parse(rest), None, "{rest:?}");
        }
    }

    #[test]
    fn an_offer_is_recorded_under_an_id_no_catalogue_entry_carries() {
        assert_eq!(exposure_id(Pillar::Fuelling), "team_offer_fuelling");
        for pillar in Pillar::ALL {
            assert!(UseCaseCatalogue::pinned()
                .get(&exposure_id(pillar))
                .is_none());
        }
    }
}
