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
//! an English wrapper around her coach's French reply, on every surface
//! including push, and switching language repaired nothing.
//!
//! The event kind is the `notification_type` column that already existed; what
//! this module adds is the parameter object stored beside it under
//! [`PARAMS_DATA_KEY`], and the catalogue keys that turn the pair back into a
//! sentence. It mirrors `pierre_memory::PredicateCode`: a closed code with a
//! `catalogue_key`, rendered by one renderer above this crate.

use serde_json::{Map, Value};

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
    /// A new activity arrived from a provider.
    ActivitySynced,
    /// Acute training load crossed the alert threshold.
    TrainingLoadAlert,
    /// The recovery score dropped below the alert threshold.
    LowRecoveryScore,
    /// The training-stress trend suggests accumulating fatigue.
    OvertrainingWarning,
    /// A personal record was detected on an activity.
    PersonalRecord,
    /// A cumulative milestone was reached.
    MilestoneReached,
    /// A fitness metric improved.
    FitnessImprovement,
    /// A coach sent the athlete a message.
    CoachMessage,
    /// A coach updated the athlete's training plan.
    PlanUpdated,
    /// A coach left a note on an activity.
    CoachFeedback,
    /// A provider sync failed.
    SyncFailure,
    /// The weekly digest of the pushes a persona floor withheld.
    PersonaDigest,
}

impl NotificationEvent {
    /// The `notification_type` value this event is stored under.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::ActivitySynced => "activity_synced",
            Self::TrainingLoadAlert => "training_load_alert",
            Self::LowRecoveryScore => "low_recovery_score",
            Self::OvertrainingWarning => "overtraining_warning",
            Self::PersonalRecord => "personal_record",
            Self::MilestoneReached => "milestone_reached",
            Self::FitnessImprovement => "fitness_improvement",
            Self::CoachMessage => "coach_message",
            Self::PlanUpdated => "plan_updated",
            Self::CoachFeedback => "coach_feedback",
            Self::SyncFailure => "sync_failure",
            Self::PersonaDigest => "persona_digest",
        }
    }

    /// The event a stored `notification_type` names, or `None` for a type no
    /// event owns.
    #[must_use]
    pub fn from_wire(wire: &str) -> Option<Self> {
        [
            Self::ActivitySynced,
            Self::TrainingLoadAlert,
            Self::LowRecoveryScore,
            Self::OvertrainingWarning,
            Self::PersonalRecord,
            Self::MilestoneReached,
            Self::FitnessImprovement,
            Self::CoachMessage,
            Self::PlanUpdated,
            Self::CoachFeedback,
            Self::SyncFailure,
            Self::PersonaDigest,
        ]
        .into_iter()
        .find(|event| event.wire() == wire)
    }

    /// The catalogue key whose text is this event's notification title.
    #[must_use]
    pub const fn title_key(self) -> &'static str {
        match self {
            Self::ActivitySynced => "notifications.event.activity_synced.title",
            Self::TrainingLoadAlert => "notifications.event.training_load_alert.title",
            Self::LowRecoveryScore => "notifications.event.low_recovery_score.title",
            Self::OvertrainingWarning => "notifications.event.overtraining_warning.title",
            Self::PersonalRecord => "notifications.event.personal_record.title",
            Self::MilestoneReached => "notifications.event.milestone_reached.title",
            Self::FitnessImprovement => "notifications.event.fitness_improvement.title",
            Self::CoachMessage => "notifications.event.coach_message.title",
            Self::PlanUpdated => "notifications.event.plan_updated.title",
            Self::CoachFeedback => "notifications.event.coach_feedback.title",
            Self::SyncFailure => "notifications.event.sync_failure.title",
            Self::PersonaDigest => "notifications.digest.title",
        }
    }

    /// The catalogue key whose text is this event's notification body.
    #[must_use]
    pub const fn body_key(self) -> &'static str {
        match self {
            Self::ActivitySynced => "notifications.event.activity_synced.body",
            Self::TrainingLoadAlert => "notifications.event.training_load_alert.body",
            Self::LowRecoveryScore => "notifications.event.low_recovery_score.body",
            Self::OvertrainingWarning => "notifications.event.overtraining_warning.body",
            Self::PersonalRecord => "notifications.event.personal_record.body",
            Self::MilestoneReached => "notifications.event.milestone_reached.body",
            Self::FitnessImprovement => "notifications.event.fitness_improvement.body",
            Self::CoachMessage => "notifications.event.coach_message.body",
            Self::PlanUpdated => "notifications.event.plan_updated.body",
            Self::CoachFeedback => "notifications.event.coach_feedback.body",
            Self::SyncFailure => "notifications.event.sync_failure.body",
            Self::PersonaDigest => "notifications.digest.body",
        }
    }

    /// The parameter names filling the title template's `{0}`, `{1}`, … slots.
    #[must_use]
    pub const fn title_params(self) -> &'static [&'static str] {
        match self {
            Self::SyncFailure => &["provider_name"],
            _ => &[],
        }
    }

    /// The parameter names filling the body template's `{0}`, `{1}`, … slots.
    #[must_use]
    pub const fn body_params(self) -> &'static [&'static str] {
        match self {
            Self::ActivitySynced => &["activity_type", "distance_display", "duration_display"],
            Self::TrainingLoadAlert => &["atl_value"],
            Self::LowRecoveryScore => &["score"],
            Self::OvertrainingWarning => &[],
            Self::PersonalRecord => &["distance_label", "time_display"],
            Self::MilestoneReached => &["value_display", "unit"],
            Self::FitnessImprovement => &["metric_name", "value_display"],
            Self::CoachMessage | Self::PlanUpdated => &["coach_name"],
            Self::CoachFeedback => &["coach_name", "activity_type"],
            Self::SyncFailure => &["error_summary"],
            Self::PersonaDigest => &["item_count"],
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

/// Action id: open the coach conversation with the composer focused.
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
    /// When true, skip the daily frequency cap (coach traffic).
    pub bypass_frequency_cap: bool,
}
