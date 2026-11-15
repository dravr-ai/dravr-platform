// ABOUTME: What the lead's first welcome in the personal thread says beyond the greeting: synced history, the goal and the why
// ABOUTME: Deterministic (no LLM): counts read off the durable activity cache Home reads, the About you facts quoted as written

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The first welcome speaks first, with figures (carnet#1528, D2).
//!
//! Home opens on the lead agent's first turn. When the athlete's history has
//! synced, the turn proves it read it: how many days it read, how many
//! sessions and in which sports, about how many a week, and the last one.
//! When the athlete told About you their goal and their why, it quotes them
//! back. It asks nothing: Getting started's goal-and-why row is the one place
//! the goal is asked.
//!
//! ## When the first-sync post already spoke
//!
//! carnet#831's first post speaks about the athlete's data once it lands. When
//! it landed in the personal thread before the team step, the welcome leaves
//! the figures — and the Garmin line they owe — to it, so the thread never
//! reads the same history twice in a row. That post's row carries no stamp of
//! its own (it is an ordinary model reply the platform composed), so it is
//! found by its ledger: the first-sync claim records when the post landed,
//! and the thread holds a model-written row from no later than that.
//!
//! ## Where the figures come from
//!
//! The durable activity cache, read the way Home's weekly volume and the
//! starters' athlete state read it: through the provider-terms filter, every
//! copy of one workout merged into one session. No provider is reached and
//! nothing is stored; the welcome only counts what the cache already holds.
//! The read runs under [`ai_scope::derived`], so a welcome built on
//! first-party-only data is stamped so.
//!
//! How far back the welcome says it read is read off the cache itself, as the
//! volume card reads it: the whole window when the cache holds an activity
//! older than it, else from the oldest stored session — so an athlete whose
//! history walk has not reached back yet is never told "90 days" over a week
//! of rows.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_contremaitre::messaging_strings::{
    format_template, MessagingStringsRegistry, KEY_AGENT_WELCOME_FIGURES_DISTANCE_TIME,
    KEY_AGENT_WELCOME_FIGURES_LAST, KEY_AGENT_WELCOME_FIGURES_PER_WEEK,
    KEY_AGENT_WELCOME_FIGURES_READ, KEY_AGENT_WELCOME_FIGURES_READ_ONE,
    KEY_AGENT_WELCOME_FIGURES_READ_UNMIXED, KEY_AGENT_WELCOME_GOAL_ONLY,
    KEY_AGENT_WELCOME_GOAL_WHY, KEY_AGENT_WELCOME_WHY_ONLY, KEY_REPLY_GARMIN_ATTRIBUTION,
};
use pierre_core::civil_time::{clock_date, local_date};
use pierre_core::constants::oauth_providers::garmin_recorded;
use pierre_core::duration_text;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    Activity, ConversationRecord, MessageRecord, PostPurpose, TenantId, UnitSystem,
};
use pierre_core::transport::TransportPolicy;
use pierre_core::untrusted::display_line;
use pierre_database::repositories::sport_type_string;
use pierre_database::RepositoryRegistry;
use pierre_memory::{FactSource, PredicateCode};
use pierre_providers::ai_scope;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_providers::registry::global_registry;
use pierre_services::activity_sports::activity_sport_label_key;
use pierre_services::athlete_clock::athlete_zone;
use pierre_services::notification_text;
use tracing::warn;
use uuid::Uuid;

/// The days of history the welcome reads back.
pub(crate) const HISTORY_WINDOW_DAYS: i64 = 90;

/// The fewest days the welcome says it read: "your last 2 days" reads as an
/// error, and a week holds every stored session of a younger history.
const MIN_DAYS_SAID: i64 = 7;

/// The fewest days over which a weekly average is said: under two weeks it
/// is one week's count dressed up as a habit.
const PER_WEEK_MIN_DAYS: i64 = 14;

/// The most cached rows the read takes. Ninety days of one athlete's
/// workouts, a copy per connection, sit far below it; a read that still fills
/// it reads its days from the oldest row it holds, so the count and the days
/// it is said over agree.
const MAX_HISTORY_ROWS: i64 = 1_000;

/// The sports the mix names, most sessions first.
const MIX_SPORTS: usize = 3;

/// The longest goal or why the welcome quotes; longer answers are cut.
const ABOUT_YOU_QUOTE_MAX_CHARS: usize = 160;

/// The onboarding facts scanned for the goal and the why, newest first.
const ABOUT_YOU_FACT_SCAN: i64 = 200;

/// What the welcome read off the athlete's synced history.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoryFigures {
    /// The days the counts cover, from [`MIN_DAYS_SAID`] to
    /// [`HISTORY_WINDOW_DAYS`].
    days: i64,
    /// Sessions in those days, every copy of one workout counted once.
    sessions: usize,
    /// The sports with a word to name them by, as their catalogue label key,
    /// with their sessions: most first, at most [`MIX_SPORTS`].
    mix: Vec<(&'static str, usize)>,
    /// The rounded sessions a week, when the days are enough to say one.
    per_week: Option<u64>,
    /// The newest session.
    last: LastSession,
    /// A session read holds Garmin-sourced data, which owes Garmin's line.
    garmin: bool,
}

/// The newest session, as the welcome names it.
#[derive(Debug, Clone, PartialEq)]
struct LastSession {
    /// Its sport's catalogue label key, when the sport has a word.
    sport: Option<&'static str>,
    /// Metres covered, when it recorded a distance.
    distance_meters: Option<f64>,
    /// Elapsed seconds, when it recorded any.
    duration_seconds: Option<u64>,
}

/// The goal and the why the athlete gave About you, as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AboutYou {
    /// What they are working toward.
    goal: Option<String>,
    /// Why they train.
    why: Option<String>,
}

/// The templates the first welcome's own lines are rendered from, in one
/// locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FirstWelcomeTemplates {
    /// `{0}` = days, `{1}` = sessions, `{2}` = the mix.
    read: String,
    /// `{0}` = days, `{1}` = sessions.
    read_unmixed: String,
    /// `{0}` = days.
    read_one: String,
    /// `{0}` = sessions a week.
    per_week: String,
    /// `{0}` = the last session's sport and figures.
    last: String,
    /// `{0}` = distance, `{1}` = time.
    distance_time: String,
    /// `{0}` = goal, `{1}` = why.
    goal_why: String,
    /// `{0}` = goal.
    goal_only: String,
    /// `{0}` = why.
    why_only: String,
    /// Garmin's attribution line.
    garmin: String,
}

impl FirstWelcomeTemplates {
    /// The templates for `locale`, with the registry's own fallback chain.
    #[must_use]
    pub(crate) fn resolve(strings: &MessagingStringsRegistry, locale: &str) -> Self {
        Self {
            read: strings.get(KEY_AGENT_WELCOME_FIGURES_READ, locale),
            read_unmixed: strings.get(KEY_AGENT_WELCOME_FIGURES_READ_UNMIXED, locale),
            read_one: strings.get(KEY_AGENT_WELCOME_FIGURES_READ_ONE, locale),
            per_week: strings.get(KEY_AGENT_WELCOME_FIGURES_PER_WEEK, locale),
            last: strings.get(KEY_AGENT_WELCOME_FIGURES_LAST, locale),
            distance_time: strings.get(KEY_AGENT_WELCOME_FIGURES_DISTANCE_TIME, locale),
            goal_why: strings.get(KEY_AGENT_WELCOME_GOAL_WHY, locale),
            goal_only: strings.get(KEY_AGENT_WELCOME_GOAL_ONLY, locale),
            why_only: strings.get(KEY_AGENT_WELCOME_WHY_ONLY, locale),
            garmin: strings.get(KEY_REPLY_GARMIN_ATTRIBUTION, locale),
        }
    }
}

/// How the first welcome's figures read in one athlete's language: the sport
/// labels the app prints, the decimal separator and the unit system.
pub(crate) struct Wording<'a> {
    /// The catalogue the sport labels are read from.
    pub strings: &'a MessagingStringsRegistry,
    /// The athlete's locale.
    pub locale: &'a str,
    /// Kilometres or miles.
    pub units: UnitSystem,
}

impl Wording<'_> {
    /// The app's label for the sport keyed `key`; `None` for an empty one.
    fn sport(&self, key: &str) -> Option<String> {
        Some(self.strings.get(key, self.locale)).filter(|label| !label.trim().is_empty())
    }
}

/// Read the athlete's history off the durable activity cache and count it,
/// with the policy what is derived from it must be stamped with.
///
/// `Ok((None, _))` when the cache holds no session of theirs in the window
/// the athlete's AI may read.
///
/// # Errors
///
/// Propagates the repository error of the cache read.
pub(crate) async fn read_history(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: Uuid,
    now: DateTime<Utc>,
) -> AppResult<(Option<HistoryFigures>, TransportPolicy)> {
    let (figures, policy) = ai_scope::derived(read_untracked(repos, tenant, user, now)).await;
    Ok((figures?, policy))
}

async fn read_untracked(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: Uuid,
    now: DateTime<Utc>,
) -> AppResult<Option<HistoryFigures>> {
    let registry = global_registry();
    let opens = now - Duration::days(HISTORY_WINDOW_DAYS);
    let rows = repos
        .activity_cache
        .get_cached_activities(user, &tenant, None, opens, now, MAX_HISTORY_ROWS)
        .await?;
    let read_full = usize::try_from(MAX_HISTORY_ROWS).is_ok_and(|cap| rows.len() >= cap);
    let rows = ai_scope::filter_activities(registry.as_ref(), rows);
    if rows.is_empty() {
        return Ok(None);
    }
    // The whole window is vouched for only when the cache reaches past it,
    // and only when the read itself was not cut short.
    let whole_window = !read_full && {
        let older = repos
            .activity_cache
            .get_cached_activities(user, &tenant, None, DateTime::UNIX_EPOCH, opens, 1)
            .await?;
        !ai_scope::filter_activities(registry.as_ref(), older).is_empty()
    };
    // Any copy of a workout may carry the Garmin data a merged session's
    // figures came from, so the line is owed off the copies.
    let garmin = rows
        .iter()
        .any(|activity| garmin_recorded(activity.source(), activity.device_name()));
    let (sessions, _) = merge_duplicates(rows, &DedupConfig::from_env());
    let zone = athlete_zone(repos, user).await;
    let today = clock_date(now, zone);
    let Some(oldest) = sessions
        .iter()
        .map(|session| local_date(session.start_date(), zone))
        .min()
    else {
        return Ok(None);
    };
    let days = if whole_window {
        HISTORY_WINDOW_DAYS
    } else {
        days_since(oldest, today)
    };
    Ok(tally(&sessions, days, garmin))
}

/// The days from `oldest` to `today`, both counted, held between
/// [`MIN_DAYS_SAID`] and [`HISTORY_WINDOW_DAYS`].
fn days_since(oldest: NaiveDate, today: NaiveDate) -> i64 {
    (today.signed_duration_since(oldest).num_days() + 1).clamp(MIN_DAYS_SAID, HISTORY_WINDOW_DAYS)
}

/// The figures of `sessions` — one per workout, copies already merged — read
/// over `days`; `None` when there is no session.
fn tally(sessions: &[Activity], days: i64, garmin: bool) -> Option<HistoryFigures> {
    let newest = sessions.iter().max_by_key(|session| session.start_date())?;
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for session in sessions {
        if let Some(key) = sport_key(session) {
            *counts.entry(key).or_default() += 1;
        }
    }
    let mut mix: Vec<(&'static str, usize)> = counts.into_iter().collect();
    // Most sessions first; a tie keeps the keys' own order, so one history
    // always reads the same.
    mix.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    mix.truncate(MIX_SPORTS);
    Some(HistoryFigures {
        days,
        sessions: sessions.len(),
        mix,
        per_week: per_week(sessions.len(), days),
        last: LastSession {
            sport: sport_key(newest),
            distance_meters: newest.distance_meters().filter(|metres| *metres > 0.0),
            duration_seconds: Some(newest.duration_seconds()).filter(|seconds| *seconds > 0),
        },
        garmin,
    })
}

/// The catalogue label key of `session`'s sport, the one the app prints it
/// by; `None` for a sport the vocabulary has no word for.
fn sport_key(session: &Activity) -> Option<&'static str> {
    sport_type_string(session).and_then(|sport| activity_sport_label_key(&sport))
}

/// `sessions` over `days` as a rounded weekly count: `None` under
/// [`PER_WEEK_MIN_DAYS`], for a single session, or when it rounds to none.
fn per_week(sessions: usize, days: i64) -> Option<u64> {
    if sessions < 2 || days < PER_WEEK_MIN_DAYS {
        return None;
    }
    let sessions = u64::try_from(sessions).ok()?;
    let days = u64::try_from(days).ok()?;
    // Rounded half up, in integers: (2·7·sessions + days) / (2·days).
    let weekly = (sessions * 14 + days) / (days * 2);
    (weekly > 0).then_some(weekly)
}

/// The figures as the welcome says them: what it read, the weekly average,
/// the last session. Empty when the strings hold no words for the first
/// sentence, which the others lean on.
#[must_use]
fn history_line(
    templates: &FirstWelcomeTemplates,
    figures: &HistoryFigures,
    wording: &Wording<'_>,
) -> String {
    let days = figures.days.to_string();
    let sessions = figures.sessions.to_string();
    let mix = figures
        .mix
        .iter()
        .filter_map(|(key, count)| wording.sport(key).map(|label| format!("{label} {count}")))
        .collect::<Vec<_>>()
        .join(", ");
    let read = match (figures.sessions, mix.is_empty()) {
        (1, _) => format_template(&templates.read_one, &[&days]),
        (_, true) => format_template(&templates.read_unmixed, &[&days, &sessions]),
        (_, false) => format_template(&templates.read, &[&days, &sessions, &mix]),
    };
    if read.trim().is_empty() {
        return String::new();
    }
    let per_week = figures
        .per_week
        .map(|weekly| format_template(&templates.per_week, &[&weekly.to_string()]));
    let last = last_session(templates, &figures.last, wording)
        .map(|last| format_template(&templates.last, &[&last]));
    [Some(read), per_week, last]
        .into_iter()
        .flatten()
        .map(|sentence| sentence.trim().to_owned())
        .filter(|sentence| !sentence.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The last session as `Run, 21.4 km in 1:52:14`: its sport when it has a
/// word, then its distance in its time, either alone when the other is
/// missing. `None` when it recorded neither.
///
/// The time is the exact clock every model-facing duration uses (`H:MM:SS`):
/// the row replays into the agent's later prompts, and a rounded figure there
/// is one the agent would quote back.
fn last_session(
    templates: &FirstWelcomeTemplates,
    last: &LastSession,
    wording: &Wording<'_>,
) -> Option<String> {
    let distance = last.distance_meters.map(|metres| {
        let counted = wording.units.distance_from_km(metres / 1000.0);
        format!(
            "{} {}",
            notification_text::decimal(counted, wording.locale),
            wording.units.distance_symbol()
        )
    });
    let time = last.duration_seconds.map(duration_text::clock);
    let figures = match (distance, time) {
        (Some(distance), Some(time)) => Some(format_template(
            &templates.distance_time,
            &[&distance, &time],
        )),
        (one, other) => one.or(other),
    }
    .filter(|figures| !figures.trim().is_empty())?;
    Some(match last.sport.and_then(|key| wording.sport(key)) {
        Some(sport) => format!("{sport}, {figures}"),
        None => figures,
    })
}

/// The Garmin line `figures` owe, when a session read holds Garmin data.
#[must_use]
fn garmin_line(templates: &FirstWelcomeTemplates, figures: &HistoryFigures) -> Option<String> {
    figures
        .garmin
        .then(|| templates.garmin.clone())
        .filter(|line| !line.trim().is_empty())
}

/// Read the goal and the why the athlete gave About you, newest answer of
/// each. Best-effort: unreadable facts read as no answer, and the welcome
/// quotes nothing.
pub(crate) async fn read_about_you(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: &str,
) -> AboutYou {
    let facts = match repos
        .memory
        .list_user_facts_by_source(
            tenant,
            user,
            FactSource::Onboarding,
            ABOUT_YOU_FACT_SCAN,
            ai_scope::readable_policy(),
        )
        .await
    {
        Ok(facts) => facts,
        Err(e) => {
            warn!(error = %e, "About you unreadable; the welcome quotes no goal");
            return AboutYou::default();
        }
    };
    let answer = |code: PredicateCode| {
        facts
            .iter()
            .find(|fact| fact.predicate_code == code)
            .map(|fact| display_line(&fact.object, ABOUT_YOU_QUOTE_MAX_CHARS))
            .filter(|text| !text.trim().is_empty())
    };
    AboutYou {
        goal: answer(PredicateCode::WorkingToward),
        why: answer(PredicateCode::TrainBecause),
    }
}

/// The athlete's goal and why, quoted as they wrote them; `None` when they
/// gave neither.
#[must_use]
fn about_line(templates: &FirstWelcomeTemplates, about: &AboutYou) -> Option<String> {
    let line = match (about.goal.as_deref(), about.why.as_deref()) {
        (Some(goal), Some(why)) => format_template(&templates.goal_why, &[goal, why]),
        (Some(goal), None) => format_template(&templates.goal_only, &[goal]),
        (None, Some(why)) => format_template(&templates.why_only, &[why]),
        (None, None) => return None,
    };
    Some(line).filter(|line| !line.trim().is_empty())
}

/// What the first welcome says around the team.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FirstWelcomeLines {
    /// Said under the greeting: the history it read, then the goal and the why.
    pub opening: Vec<String>,
    /// Said last: the Garmin line the figures owe.
    pub closing: Vec<String>,
}

/// The first welcome's own lines for `history` and `about`.
///
/// When `first_sync_spoke` — carnet#831's first post already spoke about the
/// same data in this thread — the figures are left to it, and so is the
/// Garmin line they owe; the goal and the why are still quoted.
#[must_use]
pub(crate) fn first_welcome_lines(
    templates: &FirstWelcomeTemplates,
    history: Option<&HistoryFigures>,
    first_sync_spoke: bool,
    about: &AboutYou,
    wording: &Wording<'_>,
) -> FirstWelcomeLines {
    let said = history.filter(|_| !first_sync_spoke);
    let opening = [
        said.map(|figures| history_line(templates, figures, wording)),
        about_line(templates, about),
    ];
    let closing = said.and_then(|figures| garmin_line(templates, figures));
    FirstWelcomeLines {
        opening: opening
            .into_iter()
            .flatten()
            .filter(|line| !line.trim().is_empty())
            .collect(),
        closing: closing.into_iter().collect(),
    }
}

/// How many of the thread's newest rows are searched for the first-sync post.
/// The first welcome is written while the thread is new: the post, a divider
/// or two, the athlete's first words at most.
const FIRST_SYNC_SCAN_ROWS: i64 = 50;

/// How far after the ledger's posted instant the post's own row may be
/// stamped: the row is written before the claim is marked, by clocks that need
/// not agree to the millisecond.
const FIRST_SYNC_ROW_SLACK_SECS: i64 = 60;

/// Whether `rows` hold carnet#831's first-sync post, which the ledger says
/// landed at `posted_at`: a model-written assistant row — not a welcome, a
/// command's answer, a divider or the Guardian's own reply — stamped no later
/// than that instant.
#[must_use]
pub(crate) fn holds_first_sync_post(rows: &[MessageRecord], posted_at: DateTime<Utc>) -> bool {
    let latest = posted_at + Duration::seconds(FIRST_SYNC_ROW_SLACK_SECS);
    rows.iter().any(|row| {
        row.role == "assistant"
            && !row.is_agent_welcome()
            && !row.is_command_turn()
            && !row.is_divider()
            && !row.is_guardian_reply()
            && DateTime::parse_from_rfc3339(&row.created_at)
                .is_ok_and(|created| created.with_timezone(&Utc) <= latest)
    })
}

/// Whether carnet#831's first-sync post already spoke in `conversation`, the
/// athlete's personal thread. Best-effort: a ledger or a thread that cannot be
/// read is logged and reads as not spoken, so the figures are said — at worst
/// twice, never lost.
pub(crate) async fn first_sync_spoke(
    repos: &RepositoryRegistry,
    athlete_tenant: TenantId,
    athlete: Uuid,
    conversation: &ConversationRecord,
    conversation_tenant: TenantId,
) -> bool {
    let posted_at = match repos
        .platform_post_claims
        .platform_post_posted_at(athlete_tenant, athlete, PostPurpose::FirstSync.as_str())
        .await
    {
        Ok(Some(posted_at)) => posted_at,
        Ok(None) => return false,
        Err(e) => {
            warn!(error = %e, "first-sync ledger unreadable; the welcome says its figures");
            return false;
        }
    };
    match repos
        .chat
        .get_recent_messages(
            &conversation.id,
            &athlete.to_string(),
            conversation_tenant,
            FIRST_SYNC_SCAN_ROWS,
            ai_scope::readable_policy(),
        )
        .await
    {
        Ok(rows) => holds_first_sync_post(&rows, posted_at),
        Err(e) => {
            warn!(error = %e, conversation_id = %conversation.id, "personal thread unreadable; the welcome says its figures");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, SportType};

    fn templates() -> FirstWelcomeTemplates {
        FirstWelcomeTemplates {
            read: "Read {0} days: {1} sessions ({2}).".to_owned(),
            read_unmixed: "Read {0} days: {1} sessions.".to_owned(),
            read_one: "Read {0} days: 1 session.".to_owned(),
            per_week: "About {0} a week.".to_owned(),
            last: "Last: {0}.".to_owned(),
            distance_time: "{0} in {1}".to_owned(),
            goal_why: "Goal “{0}”, why “{1}”.".to_owned(),
            goal_only: "Goal “{0}”.".to_owned(),
            why_only: "Why “{0}”.".to_owned(),
            garmin: "Garmin line.".to_owned(),
        }
    }

    fn session(
        id: &str,
        sport: SportType,
        day: u32,
        seconds: u64,
        metres: Option<f64>,
    ) -> Activity {
        let start = Utc
            .with_ymd_and_hms(2026, 10, day, 7, 0, 0)
            .single()
            .unwrap_or_default();
        let mut builder = ActivityBuilder::new(
            id.to_owned(),
            id.to_owned(),
            sport,
            start,
            seconds,
            "strava".to_owned(),
        );
        if let Some(metres) = metres {
            builder = builder.distance_meters(metres);
        }
        builder.build()
    }

    fn english() -> MessagingStringsRegistry {
        MessagingStringsRegistry::new()
    }

    fn wording(strings: &MessagingStringsRegistry, units: UnitSystem) -> Wording<'_> {
        Wording {
            strings,
            locale: "en",
            units,
        }
    }

    fn label(strings: &MessagingStringsRegistry, key: &str) -> String {
        strings.get(key, "en")
    }

    #[test]
    fn the_days_said_run_from_the_oldest_session_held_between_a_week_and_the_window() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap_or_default();
        let back = |days: i64| today - Duration::days(days);
        assert_eq!(days_since(back(0), today), MIN_DAYS_SAID);
        assert_eq!(days_since(back(19), today), 20);
        assert_eq!(days_since(back(400), today), HISTORY_WINDOW_DAYS);
    }

    #[test]
    fn a_weekly_average_needs_two_weeks_and_two_sessions() {
        assert_eq!(per_week(46, 90), Some(4), "46 in 90 days is 3.6 a week");
        assert_eq!(per_week(10, 13), None, "under two weeks");
        assert_eq!(per_week(1, 90), None, "one session is no habit");
        assert_eq!(per_week(2, 90), None, "rounds to none");
        assert_eq!(per_week(7, 14), Some(4), "3.5 rounds up");
    }

    #[test]
    fn the_mix_counts_workouts_by_the_apps_own_label_most_first() {
        let sessions = [
            session("a", SportType::Run, 1, 1_800, Some(5_000.0)),
            session("b", SportType::Ride, 2, 3_600, Some(30_000.0)),
            session("c", SportType::Run, 3, 1_800, Some(5_000.0)),
            session("d", SportType::Swim, 4, 1_800, Some(1_500.0)),
            session("e", SportType::Run, 5, 1_800, Some(5_000.0)),
            session("f", SportType::Yoga, 6, 1_800, None),
            session("g", SportType::Ride, 7, 3_600, Some(30_000.0)),
        ];
        let figures = tally(&sessions, 30, false).expect("sessions");
        assert_eq!(figures.sessions, 7);
        let keys: Vec<(&str, usize)> = figures.mix.clone();
        assert_eq!(keys.len(), MIX_SPORTS, "three sports named at most");
        assert_eq!(keys[0], ("app.sportRun", 3));
        assert_eq!(keys[1], ("app.sportRide", 2));
        assert_eq!(figures.per_week, Some(2));
        assert_eq!(
            figures.last.sport,
            Some("app.sportRide"),
            "the newest is day 7"
        );
    }

    #[test]
    fn the_line_reads_the_count_the_mix_the_week_and_the_last_session() {
        let strings = english();
        let sessions = [
            session("a", SportType::Run, 1, 1_800, Some(5_000.0)),
            session("b", SportType::Run, 3, 6_734, Some(21_400.0)),
        ];
        let figures = tally(&sessions, 20, false).expect("sessions");
        let line = history_line(
            &templates(),
            &figures,
            &wording(&strings, UnitSystem::Metric),
        );
        let run = label(&strings, "app.sportRun");
        assert_eq!(
            line,
            format!("Read 20 days: 2 sessions ({run} 2). About 1 a week. Last: {run}, 21.4 km in 1:52:14.")
        );
    }

    #[test]
    fn one_session_is_said_as_one_and_miles_for_an_imperial_athlete() {
        let strings = english();
        let sessions = [session("a", SportType::Run, 3, 3_600, Some(16_093.44))];
        let figures = tally(&sessions, 7, false).expect("a session");
        let line = history_line(
            &templates(),
            &figures,
            &wording(&strings, UnitSystem::Imperial),
        );
        let run = label(&strings, "app.sportRun");
        assert_eq!(
            line,
            format!("Read 7 days: 1 session. Last: {run}, 10.0 mi in 1:00:00.")
        );
    }

    #[test]
    fn a_sport_without_a_word_is_counted_but_never_named() {
        let strings = english();
        let other = || SportType::Other("Pickleball <b>".to_owned());
        let sessions = [
            session("a", other(), 1, 1_800, None),
            session("b", other(), 2, 2_700, None),
        ];
        let figures = tally(&sessions, 14, false).expect("sessions");
        let line = history_line(
            &templates(),
            &figures,
            &wording(&strings, UnitSystem::Metric),
        );
        assert_eq!(
            line,
            "Read 14 days: 2 sessions. About 1 a week. Last: 0:45:00."
        );
        assert!(!line.contains("Pickleball"), "{line}");
    }

    #[test]
    fn a_decimal_comma_where_the_locale_writes_one() {
        let strings = english();
        let sessions = [session("a", SportType::Ride, 3, 3_600, Some(42_195.0))];
        let figures = tally(&sessions, 7, false).expect("a session");
        let french = Wording {
            strings: &strings,
            locale: "fr",
            units: UnitSystem::Metric,
        };
        assert!(history_line(&templates(), &figures, &french).contains("42,2 km in 1:00:00"));
    }

    #[test]
    fn no_words_for_the_first_sentence_says_nothing() {
        let strings = english();
        let mut blank = templates();
        blank.read_one = String::new();
        let sessions = [session("a", SportType::Run, 3, 1_800, None)];
        let figures = tally(&sessions, 7, false).expect("a session");
        assert_eq!(
            history_line(&blank, &figures, &wording(&strings, UnitSystem::Metric)),
            ""
        );
    }

    #[test]
    fn garmin_data_owes_the_line_and_nothing_else_does() {
        let sessions = [session("a", SportType::Run, 3, 1_800, None)];
        let plain = tally(&sessions, 7, false).expect("a session");
        let garmin = tally(&sessions, 7, true).expect("a session");
        assert_eq!(garmin_line(&templates(), &plain), None);
        assert_eq!(
            garmin_line(&templates(), &garmin).as_deref(),
            Some("Garmin line.")
        );
    }

    #[test]
    fn the_goal_and_the_why_are_quoted_as_given() {
        let both = AboutYou {
            goal: Some("Montreal marathon under 3:45".to_owned()),
            why: Some("my father ran it".to_owned()),
        };
        assert_eq!(
            about_line(&templates(), &both).as_deref(),
            Some("Goal “Montreal marathon under 3:45”, why “my father ran it”.")
        );
        let goal = AboutYou {
            goal: Some("Montreal marathon under 3:45".to_owned()),
            why: None,
        };
        assert_eq!(
            about_line(&templates(), &goal).as_deref(),
            Some("Goal “Montreal marathon under 3:45”.")
        );
        let why = AboutYou {
            goal: None,
            why: Some("my father ran it".to_owned()),
        };
        assert_eq!(
            about_line(&templates(), &why).as_deref(),
            Some("Why “my father ran it”.")
        );
        assert_eq!(about_line(&templates(), &AboutYou::default()), None);
    }

    /// Two sessions read, one of them on a Garmin watch.
    fn garmin_history() -> HistoryFigures {
        let sessions = [
            session("a", SportType::Run, 1, 1_800, Some(5_000.0)),
            session("b", SportType::Run, 3, 2_400, Some(8_000.0)),
        ];
        tally(&sessions, 20, true).expect("sessions")
    }

    fn goal() -> AboutYou {
        AboutYou {
            goal: Some("A spring half".to_owned()),
            why: None,
        }
    }

    // carnet#1528, Phil 2026-11-14: the figures are said unless carnet#831's
    // first post already spoke about the same data in the thread.
    #[test]
    fn the_figures_and_their_garmin_line_are_said_while_the_first_sync_post_has_not_spoken() {
        let strings = english();
        let wording = wording(&strings, UnitSystem::Metric);
        let history = garmin_history();
        let lines = first_welcome_lines(&templates(), Some(&history), false, &goal(), &wording);
        assert_eq!(lines.opening.len(), 2, "{lines:?}");
        assert!(
            lines.opening[0].starts_with("Read 20 days: 2 sessions"),
            "{lines:?}"
        );
        assert_eq!(lines.opening[1], "Goal “A spring half”.");
        assert_eq!(lines.closing, ["Garmin line."]);
    }

    #[test]
    fn the_figures_and_their_garmin_line_are_left_to_the_first_sync_post_once_it_spoke() {
        let strings = english();
        let wording = wording(&strings, UnitSystem::Metric);
        let history = garmin_history();
        let lines = first_welcome_lines(&templates(), Some(&history), true, &goal(), &wording);
        assert_eq!(
            lines.opening,
            ["Goal “A spring half”."],
            "the goal is still quoted"
        );
        assert!(lines.closing.is_empty(), "no figures, so no Garmin line");

        let nothing =
            first_welcome_lines(&templates(), None, false, &AboutYou::default(), &wording);
        assert_eq!(nothing, FirstWelcomeLines::default());
    }

    fn row(role: &str, finish_reason: Option<&str>, created_at: &str) -> MessageRecord {
        MessageRecord {
            id: format!("{role}-{created_at}"),
            conversation_id: "conv-home".to_owned(),
            role: role.to_owned(),
            content: "Your Strava data synced.".to_owned(),
            token_count: None,
            prompt_tokens: None,
            model: None,
            finish_reason: finish_reason.map(str::to_owned),
            content_blocks: None,
            created_at: created_at.to_owned(),
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        }
    }

    #[test]
    fn the_first_sync_post_is_a_model_reply_from_no_later_than_the_ledger_says_it_landed() {
        let posted = Utc
            .with_ymd_and_hms(2026, 11, 14, 9, 0, 30)
            .single()
            .unwrap_or_default();
        let post = row("assistant", Some("stop"), "2026-11-14T09:00:29Z");
        assert!(holds_first_sync_post(&[post], posted));
        let skewed = row("assistant", Some("stop"), "2026-11-14T09:01:00Z");
        assert!(
            holds_first_sync_post(&[skewed], posted),
            "within the clocks' slack"
        );

        let later = row("assistant", Some("stop"), "2026-11-14T09:05:00Z");
        let welcome = row("assistant", Some("agent_welcome"), "2026-11-14T08:59:00Z");
        let command = row("assistant", Some("command"), "2026-11-14T08:59:00Z");
        let divider = row("system", Some("divider"), "2026-11-14T08:58:00Z");
        let athlete = row("user", None, "2026-11-14T08:59:00Z");
        let unparsed = row("assistant", Some("stop"), "yesterday");
        assert!(
            !holds_first_sync_post(
                &[later, welcome, command, divider, athlete, unparsed],
                posted
            ),
            "none of these is the first-sync post"
        );
        assert!(!holds_first_sync_post(&[], posted));
    }
}
