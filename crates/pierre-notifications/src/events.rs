// ABOUTME: The closed set of product events a notification can carry, and the catalogue keys naming them
// ABOUTME: A stored row keeps the event kind plus its parameters, never a sentence in one language

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Notification events
//!
//! A notification row records *what happened* — an event kind plus the
//! parameters that describe it — and the sentence is rendered per locale at
//! read time from the string catalogue. Writing the sentence at creation time
//! froze one language into the database: an athlete whose locale is `fr` read
//! an English wrapper around her agent's French reply, on every surface
//! including push, and switching language repaired nothing.
//!
//! The event kind is the `notification_type` column that already existed; what
//! this module adds is the parameter object stored beside it under
//! [`PARAMS_DATA_KEY`], and the catalogue keys that turn the pair back into a
//! sentence. It mirrors `pierre_memory::PredicateCode`: a closed code with a
//! `catalogue_key`, rendered by one renderer above this crate.

use pierre_core::transport::TransportPolicy;
use serde_json::{json, Map, Value};

use uuid::Uuid;

use crate::models::{NotificationActionType, NotificationCategory};
use crate::TenantId;

/// Key under a notification's `data` object holding the event's parameters.
///
/// Its presence is what marks a row as event-rendered: a row that carries it
/// renders from the catalogue in the reader's locale, and one that does not
/// predates the event vocabulary and keeps the text it was stored with.
pub const PARAMS_DATA_KEY: &str = "params";

/// A product event that raises a notification.
///
/// The wire form is the `notification_type` value persisted on the row, so the
/// set is closed by the same strings the clients already route on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationEvent {
    /// Acute training load crossed the alert threshold.
    TrainingLoadAlert,
    /// The recovery score dropped below the alert threshold.
    LowRecoveryScore,
    /// The training-stress trend suggests accumulating fatigue.
    OvertrainingWarning,
    /// A synced run set an all-time best effort at a standard distance.
    PersonalRecord,
    /// A fitness metric improved.
    FitnessImprovement,
    /// An agent sent the athlete a message.
    AgentMessage,
    /// An administrator edited a system agent the athlete is assigned.
    AgentUpdated,
    /// An administrator assigned the athlete a system agent.
    AgentAssigned,
    /// A provider sync failed.
    SyncFailure,
    /// The athlete's shared Strava seat will be released unless they come
    /// back: the seat-reclaim sweeper's warning before it disconnects them.
    SeatReleaseWarning,
    /// A group's coach asked to read a member's `TrainingPeaks` workouts
    /// through the coach's own `TrainingPeaks` account: sent to the member,
    /// whose confirmation is the consent to that read.
    DelegationProposed,
    /// The member confirmed the coach's link: sent to the coach.
    DelegationConfirmed,
    /// The member declined the coach's link: sent to the coach.
    DelegationDeclined,
    /// `TrainingPeaks` no longer lists a linked member on the coach's roster,
    /// so the link ended: sent to the coach.
    DelegationOffRoster,
    /// The same end, sent to the member: their workouts are no longer read
    /// through the coach's account.
    DelegationOffCoachRoster,
    /// The weekly digest of the pushes a persona floor withheld.
    PersonaDigest,
    /// The daily digest of the pushes a persona floor withheld.
    PersonaDailyDigest,
    /// The digest of the pushes a persona floor withheld since the athlete's
    /// previous training session, sent when their next one lands.
    PersonaSessionDigest,
    /// The digest of the pushes a persona floor withheld about one athlete,
    /// sent to the coach they concern — one per athlete.
    PersonaAthleteDigest,
    /// A coaching group's weekly roll-up, sent to the members who manage it.
    ///
    /// Its body is the summary line [`Self::body_params`] fills, followed by
    /// the trend and one line per member drawn from the `trend`, `members`,
    /// `highlights` and `concerns` parameters.
    GroupWeeklyDigest,
}

impl NotificationEvent {
    /// The `notification_type` value this event is stored under.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::TrainingLoadAlert => "training_load_alert",
            Self::LowRecoveryScore => "low_recovery_score",
            Self::OvertrainingWarning => "overtraining_warning",
            Self::PersonalRecord => "personal_record",
            Self::FitnessImprovement => "fitness_improvement",
            Self::AgentMessage => "coach_message",
            Self::AgentUpdated => "agent_updated",
            Self::AgentAssigned => "agent_assigned",
            Self::SyncFailure => "sync_failure",
            Self::SeatReleaseWarning => "seat_release_warning",
            Self::DelegationProposed => "delegation_proposed",
            Self::DelegationConfirmed => "delegation_confirmed",
            Self::DelegationDeclined => "delegation_declined",
            Self::DelegationOffRoster => "delegation_off_roster",
            Self::DelegationOffCoachRoster => "delegation_off_coach_roster",
            Self::PersonaDigest => "persona_digest",
            Self::PersonaDailyDigest => "persona_daily_digest",
            Self::PersonaSessionDigest => "persona_session_digest",
            Self::PersonaAthleteDigest => "persona_athlete_digest",
            Self::GroupWeeklyDigest => "group_weekly_digest",
        }
    }

    /// The event a stored `notification_type` names, or `None` for a type no
    /// event owns.
    #[must_use]
    pub fn from_wire(wire: &str) -> Option<Self> {
        [
            Self::TrainingLoadAlert,
            Self::LowRecoveryScore,
            Self::OvertrainingWarning,
            Self::PersonalRecord,
            Self::FitnessImprovement,
            Self::AgentMessage,
            Self::AgentUpdated,
            Self::AgentAssigned,
            Self::SyncFailure,
            Self::SeatReleaseWarning,
            Self::DelegationProposed,
            Self::DelegationConfirmed,
            Self::DelegationDeclined,
            Self::DelegationOffRoster,
            Self::DelegationOffCoachRoster,
            Self::PersonaDigest,
            Self::PersonaDailyDigest,
            Self::PersonaSessionDigest,
            Self::PersonaAthleteDigest,
            Self::GroupWeeklyDigest,
        ]
        .into_iter()
        .find(|event| event.wire() == wire)
    }

    /// The catalogue key whose text is this event's notification title.
    #[must_use]
    pub const fn title_key(self) -> &'static str {
        match self {
            Self::TrainingLoadAlert => "notifications.event.training_load_alert.title",
            Self::LowRecoveryScore => "notifications.event.low_recovery_score.title",
            Self::OvertrainingWarning => "notifications.event.overtraining_warning.title",
            Self::PersonalRecord => "notifications.event.personal_record.title",
            Self::FitnessImprovement => "notifications.event.fitness_improvement.title",
            Self::AgentMessage => "notifications.event.agent_message.title",
            Self::AgentUpdated => "notifications.event.agent_updated.title",
            Self::AgentAssigned => "notifications.event.agent_assigned.title",
            Self::SyncFailure => "notifications.event.sync_failure.title",
            Self::SeatReleaseWarning => "notifications.event.seat_release_warning.title",
            Self::DelegationProposed => "notifications.event.delegation_proposed.title",
            Self::DelegationConfirmed => "notifications.event.delegation_confirmed.title",
            Self::DelegationDeclined => "notifications.event.delegation_declined.title",
            Self::DelegationOffRoster => "notifications.event.delegation_off_roster.title",
            Self::DelegationOffCoachRoster => {
                "notifications.event.delegation_off_coach_roster.title"
            }
            Self::PersonaDigest => "notifications.digest.title",
            Self::PersonaDailyDigest => "notifications.digest.daily.title",
            Self::PersonaSessionDigest => "notifications.digest.session.title",
            Self::PersonaAthleteDigest => "notifications.digest.athlete.title",
            Self::GroupWeeklyDigest => "notifications.group_digest.title",
        }
    }

    /// The catalogue key whose text is this event's notification body.
    #[must_use]
    pub const fn body_key(self) -> &'static str {
        match self {
            Self::TrainingLoadAlert => "notifications.event.training_load_alert.body",
            Self::LowRecoveryScore => "notifications.event.low_recovery_score.body",
            Self::OvertrainingWarning => "notifications.event.overtraining_warning.body",
            Self::PersonalRecord => "notifications.event.personal_record.body",
            Self::FitnessImprovement => "notifications.event.fitness_improvement.body",
            Self::AgentMessage => "notifications.event.agent_message.body",
            Self::AgentUpdated => "notifications.event.agent_updated.body",
            Self::AgentAssigned => "notifications.event.agent_assigned.body",
            Self::SyncFailure => "notifications.event.sync_failure.body",
            Self::SeatReleaseWarning => "notifications.event.seat_release_warning.body",
            Self::DelegationProposed => "notifications.event.delegation_proposed.body",
            Self::DelegationConfirmed => "notifications.event.delegation_confirmed.body",
            Self::DelegationDeclined => "notifications.event.delegation_declined.body",
            Self::DelegationOffRoster => "notifications.event.delegation_off_roster.body",
            Self::DelegationOffCoachRoster => {
                "notifications.event.delegation_off_coach_roster.body"
            }
            Self::PersonaDigest => "notifications.digest.body",
            Self::PersonaDailyDigest => "notifications.digest.daily.body",
            Self::PersonaSessionDigest => "notifications.digest.session.body",
            Self::PersonaAthleteDigest => "notifications.digest.athlete.body",
            Self::GroupWeeklyDigest => "notifications.group_digest.summary",
        }
    }

    /// The parameter names filling the title template's `{0}`, `{1}`, … slots.
    #[must_use]
    pub const fn title_params(self) -> &'static [&'static str] {
        match self {
            Self::SyncFailure | Self::SeatReleaseWarning => &["provider_name"],
            Self::DelegationProposed
            | Self::DelegationConfirmed
            | Self::DelegationDeclined
            | Self::DelegationOffRoster
            | Self::DelegationOffCoachRoster => &["platform_name"],
            Self::GroupWeeklyDigest => &["group_name"],
            Self::PersonaAthleteDigest => &["athlete_name"],
            _ => &[],
        }
    }

    /// The parameter names filling the body template's `{0}`, `{1}`, … slots.
    #[must_use]
    pub const fn body_params(self) -> &'static [&'static str] {
        match self {
            Self::TrainingLoadAlert => &["atl_value"],
            Self::LowRecoveryScore => &["score"],
            Self::OvertrainingWarning => &[],
            Self::PersonalRecord => &["distance", "time_display"],
            Self::FitnessImprovement => &["metric_name", "value_display"],
            Self::AgentMessage | Self::AgentUpdated | Self::AgentAssigned => &["agent_name"],
            Self::SyncFailure => &["provider_name"],
            Self::SeatReleaseWarning => &["idle_days", "provider_name", "days_left"],
            Self::DelegationProposed | Self::DelegationOffCoachRoster => {
                &["coach_name", "group_name", "platform_name"]
            }
            Self::DelegationConfirmed | Self::DelegationDeclined | Self::DelegationOffRoster => {
                &["member_name", "group_name", "platform_name"]
            }
            Self::PersonaDigest | Self::PersonaDailyDigest | Self::PersonaSessionDigest => {
                &["item_count"]
            }
            Self::PersonaAthleteDigest => &["item_count", "athlete_name"],
            Self::GroupWeeklyDigest => &["active_members", "total_members", "avg_volume_km"],
        }
    }

    /// The title and body keys a *group* of this event renders through, when
    /// the feed collapses consecutive rows into one.
    ///
    /// Both take the group size as `{0}`. `None` for an event the feed never
    /// collapses, which is every event but [`Self::SyncFailure`].
    #[must_use]
    pub const fn collapsed_keys(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::SyncFailure => Some((
                "notifications.event.sync_failure.collapsed_title",
                "notifications.event.sync_failure.collapsed_body",
            )),
            _ => None,
        }
    }
}

/// An action button a trigger attaches to its notification.
///
/// The button's label is not carried here: it is rendered from the catalogue
/// in the reader's locale, exactly like the title and body, so a French
/// athlete never taps an English "Reply".
#[derive(Debug, Clone)]
pub struct NotificationActionSpec {
    /// Stable action id — what the client sends back, and the key the label
    /// is looked up under.
    pub id: &'static str,
    /// What tapping the button does.
    pub action_type: NotificationActionType,
}

/// Action id: open the agent conversation with the composer focused.
pub const ACTION_REPLY: &str = "reply";
/// Action id: reopen the provider connection flow.
pub const ACTION_RECONNECT: &str = "reconnect";

/// The catalogue key naming an action button, or `None` for an id the
/// catalogue has no word for — that button keeps the label it was stored
/// with rather than showing a key.
#[must_use]
pub fn action_label_key(id: &str) -> Option<&'static str> {
    match id {
        ACTION_REPLY => Some("notifications.action.reply"),
        ACTION_RECONNECT => Some("notifications.action.reconnect"),
        _ => None,
    }
}

/// Key under a notification's `data` object stamping it as derived from data
/// whose terms keep it on Dravr's own surfaces (carnet#769).
///
/// The rows are stored by `dravr-commere`, whose table this platform does not
/// add columns to, so the stamp rides in the row's own `data` object. Absent
/// on every row written before stamping existed, which reads as unstamped.
pub(crate) const FIRST_PARTY_ONLY_DATA_KEY: &str = "first_party_only";

/// `data` stamped with `policy`.
///
/// A first-party-only notification carries a `first_party_only: true` key,
/// anything else is returned as it was. A non-object payload is kept under `"payload"`, the way the persona gate
/// keeps one.
#[must_use]
pub fn stamp_data(data: Option<Value>, policy: TransportPolicy) -> Option<Value> {
    if !policy.is_first_party_only() {
        return data;
    }
    let mut object = match data {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = Map::new();
            map.insert("payload".to_owned(), other);
            map
        }
        None => Map::new(),
    };
    object.insert(FIRST_PARTY_ONLY_DATA_KEY.to_owned(), Value::Bool(true));
    Some(Value::Object(object))
}

/// The stamp a stored notification's `data` carries.
#[must_use]
pub fn data_transport_policy(data: Option<&Value>) -> TransportPolicy {
    TransportPolicy::from_first_party_only(
        data.and_then(|data| data.get(FIRST_PARTY_ONLY_DATA_KEY))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

/// Merge an event's parameters into its deep-link routing payload, producing
/// the `data` object stored on the notification row.
///
/// `route` is the screen/id payload the clients navigate on; `params` lands
/// under [`PARAMS_DATA_KEY`] so the read path can re-render the sentence.
#[must_use]
pub fn event_data(route: Value, params: Value) -> Value {
    let mut object = match route {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            if !other.is_null() {
                map.insert("route".to_owned(), other);
            }
            map
        }
    };
    object.insert(PARAMS_DATA_KEY.to_owned(), params);
    Value::Object(object)
}

/// The parameter object stored on a notification row, or `None` when the row
/// predates the event vocabulary and carries no parameters.
#[must_use]
pub fn event_params(data: Option<&Value>) -> Option<&Map<String, Value>> {
    data?.get(PARAMS_DATA_KEY)?.as_object()
}

/// Key under a notification's `data` object naming the athlete the
/// notification concerns when that athlete is not its recipient.
///
/// A coach told about one member of their group gets a row carrying it; a
/// row without it concerns its own recipient.
///
/// The `per_athlete` persona digest rolls the withheld notifications it
/// returns up on this key, one digest per athlete.
pub const SUBJECT_ATHLETE_DATA_KEY: &str = "subject_athlete";

/// The athlete a notification concerns, when that is not its recipient.
///
/// Stored under [`SUBJECT_ATHLETE_DATA_KEY`] with the name the notification
/// used, so a digest naming the athlete reads what the notifications it rolls
/// up read, without another lookup of someone else's account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectAthlete {
    /// The athlete's user id.
    pub id: Uuid,
    /// The athlete's name as the notification gave it.
    pub name: String,
}

impl SubjectAthlete {
    /// The value stored under [`SUBJECT_ATHLETE_DATA_KEY`].
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({ "id": self.id.to_string(), "name": self.name })
    }

    /// The athlete a stored notification's `data` names, or `None` when the
    /// notification concerns its own recipient.
    #[must_use]
    pub fn from_data(data: Option<&Value>) -> Option<Self> {
        let subject = data?.get(SUBJECT_ATHLETE_DATA_KEY)?;
        let id = subject.get("id")?.as_str()?.parse().ok()?;
        let name = subject.get("name")?.as_str()?.to_owned();
        Some(Self { id, name })
    }
}

/// A product event, ready to dispatch.
///
/// A trigger declares *what happened* and nothing about how it reads: the
/// facade renders the title, body and action labels through
/// [`crate::NotificationLocalizer`] in the recipient's own language, and the
/// stored row keeps the event plus its parameters so the notification centre
/// can render it again when the athlete changes language.
#[derive(Debug, Clone)]
pub struct EventDispatch {
    /// Recipient.
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation.
    pub tenant_id: TenantId,
    /// Preference bucket the dispatch pipeline suppresses on.
    pub category: NotificationCategory,
    /// What happened.
    pub event: NotificationEvent,
    /// The event's parameters, an object keyed by
    /// [`NotificationEvent::title_params`] and
    /// [`NotificationEvent::body_params`].
    pub params: Value,
    /// Deep-link routing payload — the `screen` / `id` pair the clients
    /// navigate on. Merged with `params` into the stored `data`.
    pub route: Value,
    /// Action buttons, by id; their labels are rendered per locale.
    pub actions: Option<Vec<NotificationActionSpec>>,
    /// When true, skip the daily frequency cap (agent traffic).
    pub bypass_frequency_cap: bool,
    /// The stamp of what the event was derived from (carnet#769). The facade
    /// tightens it to the derivation the dispatch runs in, so a trigger fired
    /// from inside a turn needs to name nothing.
    pub transport_policy: TransportPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_party_only_stamp_rides_in_the_data_and_reads_back() {
        let route = json!({ "screen": "coach", "id": "c1" });
        let stamped = stamp_data(Some(route.clone()), TransportPolicy::FirstPartyOnly);
        assert_eq!(
            data_transport_policy(stamped.as_ref()),
            TransportPolicy::FirstPartyOnly
        );
        assert_eq!(
            stamped.as_ref().and_then(|d| d.get("screen")),
            Some(&json!("coach")),
            "the routing payload is kept"
        );

        let unstamped = stamp_data(Some(route.clone()), TransportPolicy::AnyTransport);
        assert_eq!(unstamped, Some(route), "an open row is left as it was");
        assert_eq!(
            data_transport_policy(unstamped.as_ref()),
            TransportPolicy::AnyTransport
        );

        let from_nothing = stamp_data(None, TransportPolicy::FirstPartyOnly);
        assert_eq!(
            from_nothing,
            Some(json!({ FIRST_PARTY_ONLY_DATA_KEY: true }))
        );
        let from_scalar = stamp_data(Some(json!(7)), TransportPolicy::FirstPartyOnly);
        assert_eq!(
            from_scalar,
            Some(json!({ "payload": 7, FIRST_PARTY_ONLY_DATA_KEY: true }))
        );
        assert_eq!(
            data_transport_policy(None),
            TransportPolicy::AnyTransport,
            "a row written before stamping is unstamped"
        );
    }
}
