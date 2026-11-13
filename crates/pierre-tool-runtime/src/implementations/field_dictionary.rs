// ABOUTME: The field dictionary as the model reads it: each provider's modes on get_connection_status
// ABOUTME: and, on get_activities, what the connections that served the rows never supply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The wire shapes of [`pierre_providers::field_support`].
//!
//! `get_connection_status` carries a provider's whole dictionary, both modes
//! ([`FieldDictionary`]). `get_activities` runs on every grounded turn, so its
//! header stays small: per connection that served the rows, only the fields
//! it never supplies and those only a detail read adds ([`ServedFields`]).
//! The modes are named `mirror` and `oauth`, the words `backend` already
//! uses, never a backend's internal key.

use std::collections::BTreeSet;

use pierre_core::models::Activity;
use pierre_providers::field_support::{
    field_support, ActivityFieldSupport, FieldMode, FieldSupport,
};
use pierre_providers::registry::global_registry;
use pierre_providers::ProviderRegistry;
use schemars::JsonSchema;
use serde::Serialize;

/// What one mode supplies, by the keys the activity rows use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ModeFields {
    /// Fields every list row carries when the activity recorded them.
    pub list: Vec<&'static str>,
    /// Fields only a detail read (`mode=detailed`) adds.
    pub detail_only: Vec<&'static str>,
    /// Channels of the activity's sample streams.
    pub streams: Vec<&'static str>,
}

impl From<ActivityFieldSupport> for ModeFields {
    fn from(support: ActivityFieldSupport) -> Self {
        Self {
            list: support.list.names(),
            detail_only: support.detail_only.names(),
            streams: support.streams.names(),
        }
    }
}

/// A provider's field dictionary: what each way of reading it supplies.
///
/// A field neither mode lists is never supplied, so its absence on a row
/// means "not available from this connection", never zero or "did not
/// happen".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FieldDictionary {
    /// The mode serving the athlete: `mirror` (read by signing in as the
    /// athlete) or `oauth` (the provider's API). Absent when none serves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serving: Option<&'static str>,
    /// What the mirror supplies; absent when the provider has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<ModeFields>,
    /// What the provider's API supplies; absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<ModeFields>,
}

impl From<FieldSupport> for FieldDictionary {
    fn from(support: FieldSupport) -> Self {
        Self {
            serving: support.serving.map(FieldMode::as_str),
            mirror: support.mirror.map(ModeFields::from),
            oauth: support.oauth.map(ModeFields::from),
        }
    }
}

/// The dictionary of a user-facing `provider` served in `serving`, or `None`
/// when the provider serves no activities.
#[must_use]
pub fn dictionary(
    registry: &ProviderRegistry,
    provider: &str,
    serving: Option<FieldMode>,
) -> Option<FieldDictionary> {
    field_support(registry, provider, serving).map(FieldDictionary::from)
}

/// What one connection that served the rows never supplies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ServedFields {
    /// The provider, as the athlete names it.
    pub provider: &'static str,
    /// `mirror` or `oauth`: how it was read.
    pub via: &'static str,
    /// Fields this connection never supplies: absent on its rows means not
    /// available, never zero.
    pub not_supplied: Vec<&'static str>,
    /// Fields its rows carry only after a detail read (`mode=detailed`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub detail_only: Vec<&'static str>,
}

/// The note the `get_activities` header carries for the model.
const NOT_SUPPLIED_NOTE: &str = "A field listed under not_supplied is never provided by \
    that connection: its absence means the value is unknown, not zero, and is no evidence the \
    athlete did not record it. Fields under detail_only appear once the activity is read in \
    detail.";

/// The `get_activities` header: per connection that served `activities`,
/// what it never supplies. `None` when no served row came from a connection
/// with a declared dictionary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FieldSupportHeader {
    /// How to read `connections`.
    pub note: &'static str,
    /// One entry per connection that served rows, in name order.
    pub connections: Vec<ServedFields>,
}

/// The header for the rows of a `get_activities` answer.
#[must_use]
pub fn served_header(activities: &[Activity]) -> Option<FieldSupportHeader> {
    let registry = global_registry();
    let served: BTreeSet<(&str, FieldMode)> = activities
        .iter()
        .map(|activity| FieldMode::of_row(activity.provider(), activity.source()))
        .collect();
    let connections: Vec<ServedFields> = served
        .into_iter()
        .filter_map(|(provider, mode)| {
            let support = field_support(&registry, provider, Some(mode))?;
            let fields = support.served()?;
            Some(ServedFields {
                provider: support.provider,
                via: mode.as_str(),
                not_supplied: fields.not_supplied().names(),
                detail_only: fields.detail_only.names(),
            })
        })
        .collect();
    (!connections.is_empty()).then_some(FieldSupportHeader {
        note: NOT_SUPPLIED_NOTE,
        connections,
    })
}
