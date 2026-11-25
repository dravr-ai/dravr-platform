// ABOUTME: The resolved RFC 9396 document a grant stores — a preset narrowed by checked constraints, frozen on approval
// ABOUTME: Its typed accessors, fixed-order serialization, round trip, and the review digest an approval is bound to
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The resolved side: [`resolve`] expands a [`RequestedDetails`]' preset and
//! narrows it; [`ResolvedDetails`] is what the grant stores, the token
//! response echoes and introspection returns.

use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_core::errors::{AppError, AppResult};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::duration::{format_seconds, parse_seconds, SECONDS_PER_DAY};
use super::parse::{CheckedConstraints, RequestedDetails};
use super::{
    refused, AccessMode, Budgets, ClientClaims, Preset, Representation, Retention, Source, Tier,
};
use crate::oauth2_server::models::OAuth2Error;

/// The data-access detail's `type`, as a one-value enum so the document
/// reads it back exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DataAccessType {
    #[serde(rename = "https://pdpp.dev/data-access")]
    DataAccess,
}

/// The health-constraints detail's `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum HealthConstraintsType {
    #[serde(rename = "https://dravr.ai/pdpp/health-profile/v0/health-constraints")]
    HealthConstraints,
}

/// A preset, written by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PresetName(Preset);

impl Serialize for PresetName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.name())
    }
}

impl<'de> Deserialize<'de> for PresetName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Preset::from_name(&name)
            .map(Self)
            .ok_or_else(|| D::Error::custom(format!("unknown preset {name}")))
    }
}

/// The data window: `days` back from approval, frozen to `since`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    days: u32,
    /// The athlete's local midnight `days` before approval, in UTC; absent
    /// until the athlete approves
    #[serde(default, skip_serializing_if = "Option::is_none")]
    since: Option<Timestamp>,
}

/// The resolved data-access detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolvedAccess {
    #[serde(rename = "type")]
    kind: DataAccessType,
    source: Source,
    selection_preset: PresetName,
    access_mode: AccessMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    purpose_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    purpose_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_claims: Option<ClientClaims>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retention: Option<Retention>,
    window: Window,
}

/// The resolved health-constraints detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolvedConstraints {
    #[serde(rename = "type")]
    kind: HealthConstraintsType,
    applies_to: u32,
    critical: bool,
    max_tier: Tier,
    representation: Representation,
    /// ISO 8601, in one spelling per value
    lifetime: String,
    budgets: Budgets,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    purpose_dpv: Option<String>,
}

/// What a grant was approved for: a preset narrowed by the client's
/// constraints, serialized as the RFC 9396 array the grant stores, the token
/// response echoes and introspection returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDetails(ResolvedAccess, ResolvedConstraints);

/// Resolve `requested` for tokens bound to one of `accepted_sources`: the
/// preset expanded, every field the narrower of the preset's and the
/// client's.
///
/// # Errors
/// `invalid_authorization_details` when `source.id` is not among
/// `accepted_sources`.
pub fn resolve(
    requested: &RequestedDetails,
    accepted_sources: &[&str],
) -> Result<ResolvedDetails, OAuth2Error> {
    let RequestedDetails {
        access,
        preset,
        constraints,
    } = requested;
    let source = access
        .source
        .as_ref()
        .map_or("", |source| source.id.as_str());
    if !accepted_sources.contains(&source) {
        return Err(refused("source.id must be the resource the tokens are for"));
    }
    let mut details = preset_defaults(*preset, source);
    details.0.purpose_code.clone_from(&access.purpose_code);
    details
        .0
        .purpose_description
        .clone_from(&access.purpose_description);
    details.0.client_claims.clone_from(&access.client_claims);
    details.0.retention.clone_from(&access.retention);
    if let Some(constraints) = constraints {
        details.narrow(*preset, constraints);
    }
    Ok(details)
}

/// The preset's own details for tokens bound to `source_id`: what the
/// consent screen's picker grants for that preset.
///
/// A preset with no fixed window or per-day budget (`one_off_question`) has
/// none to grant here: the parser and the picker refuse it until a step-up
/// approval resolves its request's own window and total budget, and should
/// one reach this anyway its missing values resolve to 0, which reads and
/// serves nothing.
#[must_use]
pub fn preset_defaults(preset: Preset, source_id: &str) -> ResolvedDetails {
    let representation = if preset.raw_streams().is_empty() {
        Representation::Summary
    } else {
        Representation::SummaryAndRaw
    };
    ResolvedDetails(
        ResolvedAccess {
            kind: DataAccessType::DataAccess,
            source: Source {
                id: source_id.to_owned(),
            },
            selection_preset: PresetName(preset),
            access_mode: AccessMode::Continuous,
            purpose_code: None,
            purpose_description: None,
            client_claims: None,
            retention: None,
            window: Window {
                days: preset.window_days().unwrap_or(0),
                since: None,
            },
        },
        ResolvedConstraints {
            kind: HealthConstraintsType::HealthConstraints,
            applies_to: 0,
            critical: true,
            max_tier: preset.max_tier(),
            representation,
            lifetime: format_seconds(i64::from(preset.lifetime_cap_days()) * SECONDS_PER_DAY),
            budgets: Budgets {
                requests_per_hour: preset.default_requests_per_hour(),
                rows_per_day: preset.default_rows_per_day().unwrap_or(0),
                raw_rows_per_day: preset.default_raw_rows_per_day().unwrap_or(0),
            },
            purpose_dpv: None,
        },
    )
}

impl ResolvedDetails {
    /// Take the narrower of each preset value and the client's.
    fn narrow(&mut self, preset: Preset, constraints: &CheckedConstraints) {
        let resolved = &mut self.1;
        if let Some(tier) = constraints.max_tier {
            resolved.max_tier = resolved.max_tier.min(tier);
        }
        // A preset with no raw stream is a summary grant whatever is asked.
        if constraints.representation == Some(Representation::Summary) {
            resolved.representation = Representation::Summary;
        }
        let cap = i64::from(preset.lifetime_cap_days()) * SECONDS_PER_DAY;
        if let Some(seconds) = constraints.lifetime_seconds {
            resolved.lifetime = format_seconds(seconds.min(cap));
        }
        resolved.purpose_dpv.clone_from(&constraints.purpose_dpv);
        if let Some(budgets) = &constraints.budgets {
            let narrowed = |own: u32, asked: Option<u32>| asked.map_or(own, |asked| own.min(asked));
            let own = resolved.budgets;
            resolved.budgets = Budgets {
                requests_per_hour: narrowed(own.requests_per_hour, budgets.requests_per_hour),
                rows_per_day: narrowed(own.rows_per_day, budgets.rows_per_day),
                raw_rows_per_day: narrowed(own.raw_rows_per_day, budgets.raw_rows_per_day),
            };
        }
    }

    /// The preset the grant reads.
    #[must_use]
    pub const fn preset(&self) -> Preset {
        self.0.selection_preset.0
    }

    /// The highest tier a call under the grant may read.
    #[must_use]
    pub const fn max_tier(&self) -> Tier {
        self.1.max_tier
    }

    /// Whether the grant reads the preset's raw streams.
    #[must_use]
    pub const fn representation(&self) -> Representation {
        self.1.representation
    }

    /// The grant's budgets.
    #[must_use]
    pub const fn budgets(&self) -> Budgets {
        self.1.budgets
    }

    /// The days of data back from approval the grant reads.
    #[must_use]
    pub const fn window_days(&self) -> u32 {
        self.0.window.days
    }

    /// The earliest moment a row the grant reads may be dated; `None` until
    /// the athlete approved.
    #[must_use]
    pub const fn since(&self) -> Option<Timestamp> {
        self.0.window.since
    }

    /// How long the grant lasts from approval.
    #[must_use]
    pub fn lifetime(&self) -> Elapsed {
        Elapsed::seconds(parse_seconds(&self.1.lifetime).unwrap_or_default())
    }

    /// The client's purpose, in its own words.
    #[must_use]
    pub fn purpose_description(&self) -> Option<&str> {
        self.0.purpose_description.as_deref()
    }

    /// The client's commitments, in its own words; none when it made none.
    #[must_use]
    pub fn commitments(&self) -> &[String] {
        self.0
            .client_claims
            .as_ref()
            .map_or(&[], |claims| claims.commitments.as_slice())
    }

    /// These details approved at `now` by an athlete in `zone`: the window's
    /// `since` is their local midnight `window_days` back, in UTC, and the
    /// grant expires `lifetime` from `now`. Returns the frozen details and
    /// that expiry.
    ///
    /// # Errors
    /// An internal error when the dates fall outside the calendar.
    pub fn freeze(&self, now: Timestamp, zone: &Zone) -> AppResult<(Self, Timestamp)> {
        let out_of_range = || AppError::internal("A grant's window falls outside the calendar");
        let first_day = zone
            .date_at(now)
            .checked_sub_days(i64::from(self.0.window.days))
            .ok_or_else(out_of_range)?;
        let since = zone.start_of_day(first_day).map_err(|_| out_of_range())?;
        let expires_at = now.checked_add(self.lifetime()).ok_or_else(out_of_range)?;
        let mut frozen = self.clone();
        frozen.0.window.since = Some(since);
        Ok((frozen, expires_at))
    }

    /// These details as their terms, the window not yet frozen: what the
    /// athlete reviews, and what two grants are compared by.
    #[must_use]
    pub fn terms(&self) -> Self {
        let mut terms = self.clone();
        terms.0.window.since = None;
        terms
    }

    /// Whether `other` grants exactly what these details grant, whenever
    /// either was approved.
    #[must_use]
    pub fn same_terms(&self, other: &Self) -> bool {
        self.terms() == other.terms()
    }

    /// The stored document.
    ///
    /// # Errors
    /// An internal error if the details cannot be serialized.
    pub fn to_document(&self) -> AppResult<String> {
        serde_json::to_string(self).map_err(|e| {
            AppError::internal(format!("Failed to serialize authorization details: {e}"))
        })
    }

    /// The details a grant stored.
    ///
    /// # Errors
    /// An internal error when `document` is not a resolved document this
    /// server wrote.
    pub fn from_document(document: &str) -> AppResult<Self> {
        let details: Self = serde_json::from_str(document).map_err(|e| {
            AppError::internal(format!("A grant's authorization details do not read: {e}"))
        })?;
        if parse_seconds(&details.1.lifetime).is_none() {
            return Err(AppError::internal(
                "A grant's authorization details carry no lifetime",
            ));
        }
        Ok(details)
    }
}

/// The review digest an approval is bound to (owl §3.3).
///
/// The consent text version alone for a plain grant, else the SHA-256 of that
/// version and the details' terms, so the details a screen showed are the
/// details approved.
///
/// # Errors
/// An internal error if the details cannot be serialized.
pub fn review_digest(text_version: &str, details: Option<&ResolvedDetails>) -> AppResult<String> {
    let Some(details) = details else {
        return Ok(text_version.to_owned());
    };
    let mut digest = Sha256::new();
    digest.update(text_version.as_bytes());
    digest.update(b"\n");
    digest.update(details.terms().to_document()?.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

/// The zone an athlete's window is frozen in: their own, UTC when they set
/// none or it does not read.
#[must_use]
pub fn athlete_zone(timezone: Option<&str>) -> Zone {
    timezone
        .and_then(|name| {
            Zone::from_name(name)
                .inspect_err(|e| {
                    warn!(timezone = name, "An athlete's time zone does not read: {e}");
                })
                .ok()
        })
        .unwrap_or_else(Zone::utc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth2_server::authorization_details::tests::{access, constraints, resolved, MCP};
    use crate::oauth2_server::authorization_details::{DATA_ACCESS_TYPE, HEALTH_CONSTRAINTS_TYPE};
    use dravr_tronc::dates::CivilDate;
    use serde_json::json;

    // ============================================================================
    // Accepted
    // ============================================================================

    #[test]
    fn a_training_coach_preset_alone_resolves_to_its_own_values() {
        let details = resolved(&json!([access(&json!({}))])).unwrap();
        assert_eq!(
            details.preset(),
            Preset::TrainingCoach,
            "bug: the preset named is not the one resolved"
        );
        assert_eq!(
            details.max_tier(),
            Tier::T1,
            "bug: training_coach resolved above or below T1"
        );
        assert_eq!(
            details.representation(),
            Representation::SummaryAndRaw,
            "bug: training_coach lost its raw streams"
        );
        assert_eq!(
            details.budgets(),
            Budgets {
                requests_per_hour: 120,
                rows_per_day: 2_000,
                raw_rows_per_day: 50_000,
            },
            "bug: training_coach's own budgets were not kept"
        );
        assert_eq!(
            details.lifetime(),
            Elapsed::days(30),
            "bug: training_coach's lifetime cap was not kept"
        );
        assert_eq!(
            details.window_days(),
            90,
            "bug: training_coach's data window was not kept"
        );
        assert_eq!(
            details.since(),
            None,
            "bug: a request's details carry a frozen window before approval"
        );
    }

    #[test]
    fn a_sleep_coach_preset_is_a_summary_grant_whatever_is_asked() {
        let details = resolved(&json!([
            access(&json!({"selection_preset": "sleep_coach"})),
            constraints(&json!({"representation": "summary_and_raw"})),
        ]))
        .unwrap();
        assert_eq!(
            details.preset(),
            Preset::SleepCoach,
            "bug: the preset named is not the one resolved"
        );
        assert_eq!(
            details.representation(),
            Representation::Summary,
            "bug: sleep_coach, which has no raw stream, resolved to raw"
        );
        assert_eq!(
            details.budgets().requests_per_hour,
            60,
            "bug: sleep_coach's own hourly budget was not kept"
        );
    }

    #[test]
    fn the_client_words_are_kept_as_its_own() {
        let details = resolved(&json!([
            access(&json!({
                "purpose_code": "https://w3id.org/dpv#ServiceProvision",
                "purpose_description": "Plan training from your load and sleep",
                "access_mode": "continuous",
                "retention": {"max_duration": "P90D", "on_expiry": "delete"},
                "client_claims": {"commitments": ["Used only for your plan"]},
            })),
            constraints(&json!({
                "purpose_dpv": "https://w3id.org/dpv#ServiceProvision",
                "step_up": {"required": false},
                "host_profile_required": "none",
            })),
        ]))
        .unwrap();
        assert_eq!(
            details.purpose_description(),
            Some("Plan training from your load and sleep"),
            "bug: the client's purpose was not kept in its own words"
        );
        assert_eq!(
            details.commitments(),
            ["Used only for your plan"],
            "bug: the client's commitments were not kept in its own words"
        );
    }

    // ============================================================================
    // Narrow only
    // ============================================================================

    #[test]
    fn every_field_takes_the_narrower_value() {
        let narrowed = resolved(&json!([
            access(&json!({})),
            constraints(&json!({
                "max_tier": "T0",
                "representation": "summary",
                "lifetime": "P7D",
                "budgets": {"requests_per_hour": 10, "rows_per_day": 100, "raw_rows_per_day": 1000},
            })),
        ]))
        .unwrap();
        assert_eq!(
            narrowed.max_tier(),
            Tier::T0,
            "bug: a narrower tier was not taken"
        );
        assert_eq!(
            narrowed.representation(),
            Representation::Summary,
            "bug: a summary representation was not taken"
        );
        assert_eq!(
            narrowed.lifetime(),
            Elapsed::days(7),
            "bug: a shorter lifetime was not taken"
        );
        assert_eq!(
            narrowed.budgets(),
            Budgets {
                requests_per_hour: 10,
                rows_per_day: 100,
                raw_rows_per_day: 1_000,
            },
            "bug: narrower budgets were not taken"
        );
    }

    #[test]
    fn a_wider_value_never_widens_the_preset() {
        let widened = resolved(&json!([
            access(&json!({})),
            constraints(&json!({
                "max_tier": "T1",
                "lifetime": "P180D",
                "budgets": {"requests_per_hour": 10_000, "rows_per_day": 9_999_999, "raw_rows_per_day": 9_999_999},
            })),
        ]))
        .unwrap();
        assert_eq!(
            widened,
            resolved(&json!([access(&json!({}))])).unwrap(),
            "bug: a wider value widened the preset"
        );
    }

    #[test]
    fn a_budget_the_client_leaves_out_keeps_the_preset_value() {
        let details = resolved(&json!([
            access(&json!({})),
            constraints(&json!({"budgets": {"rows_per_day": 5}})),
        ]))
        .unwrap();
        assert_eq!(
            details.budgets().rows_per_day,
            5,
            "bug: the client's narrower row budget was not taken"
        );
        assert_eq!(
            details.budgets().requests_per_hour,
            120,
            "bug: a budget the client left out lost the preset value"
        );
        assert_eq!(
            details.budgets().raw_rows_per_day,
            50_000,
            "bug: a budget the client left out lost the preset value"
        );
    }

    #[test]
    fn a_lifetime_in_hours_is_kept_exactly() {
        let details = resolved(&json!([
            access(&json!({})),
            constraints(&json!({"lifetime": "PT1H"})),
        ]))
        .unwrap();
        assert_eq!(
            details.lifetime(),
            Elapsed::hours(1),
            "bug: a lifetime in hours was rounded"
        );
    }

    // ============================================================================
    // The window freeze and the document
    // ============================================================================

    #[test]
    fn the_window_freezes_at_the_athletes_local_midnight() {
        let details = resolved(&json!([access(&json!({}))])).unwrap();
        // 2026-11-22 03:30 UTC is still 2026-11-21 in Toronto (UTC-5).
        let now = Timestamp::parse_from_rfc3339("2026-11-22T03:30:00Z").unwrap();
        let toronto = athlete_zone(Some("America/Toronto"));
        let (frozen, expires_at) = details.freeze(now, &toronto).unwrap();
        let first_day = CivilDate::from_ymd(2026, 11, 21)
            .unwrap()
            .checked_sub_days(90)
            .unwrap();
        assert_eq!(
            frozen.since(),
            Some(toronto.start_of_day(first_day).unwrap()),
            "bug: the window did not freeze at the athlete's local midnight"
        );
        // 2026-08-23 local midnight is 04:00 UTC (EDT, UTC-4).
        assert_eq!(
            frozen.since().map(Timestamp::to_rfc3339),
            Some(
                Timestamp::parse_from_rfc3339("2026-08-23T04:00:00Z")
                    .unwrap()
                    .to_rfc3339()
            ),
            "bug: the frozen window is not 04:00 UTC on 2026-08-23"
        );
        assert_eq!(
            expires_at,
            now.checked_add(Elapsed::days(30)).unwrap(),
            "bug: the grant does not expire its lifetime after approval"
        );
    }

    #[test]
    fn an_athlete_with_no_zone_freezes_at_utc_midnight() {
        let details = resolved(&json!([access(&json!({}))])).unwrap();
        let now = Timestamp::parse_from_rfc3339("2026-11-22T03:30:00Z").unwrap();
        for zone in [athlete_zone(None), athlete_zone(Some("Not/AZone"))] {
            let (frozen, _) = details.freeze(now, &zone).unwrap();
            assert_eq!(
                frozen.since(),
                Some(Timestamp::parse_from_rfc3339("2026-08-24T00:00:00Z").unwrap()),
                "bug: an athlete without a readable zone froze off UTC midnight"
            );
        }
    }

    #[test]
    fn the_document_is_the_rfc_9396_array_in_a_fixed_order() {
        let details = resolved(&json!([access(&json!({}))])).unwrap();
        let document = details.to_document().unwrap();
        assert_eq!(
            document,
            format!(
                "[{{\"type\":\"{DATA_ACCESS_TYPE}\",\"source\":{{\"id\":\"{MCP}\"}},\"selection_preset\":\"training_coach\",\"access_mode\":\"continuous\",\"window\":{{\"days\":90}}}},{{\"type\":\"{HEALTH_CONSTRAINTS_TYPE}\",\"applies_to\":0,\"critical\":true,\"max_tier\":\"T1\",\"representation\":\"summary_and_raw\",\"lifetime\":\"P30D\",\"budgets\":{{\"requests_per_hour\":120,\"rows_per_day\":2000,\"raw_rows_per_day\":50000}}}}]"
            )
, "bug: the stored document's fields or their order changed");
        assert!(
            !document.contains("health.vault"),
            "bug: the document names a stream"
        );
    }

    #[test]
    fn a_frozen_document_reads_back_whole() {
        let details = resolved(&json!([access(
            &json!({"purpose_description": "Coach me"})
        )]))
        .unwrap();
        let (frozen, _) = details.freeze(Timestamp::now(), &Zone::utc()).unwrap();
        let read = ResolvedDetails::from_document(&frozen.to_document().unwrap()).unwrap();
        assert_eq!(
            read, frozen,
            "bug: a frozen document did not read back whole"
        );
        assert!(read.same_terms(&details), "freezing changes no term");
        assert_ne!(read, details, "but it does set since");
    }

    #[test]
    fn a_document_this_server_did_not_write_does_not_read() {
        for document in [
            "[]",
            "{}",
            r#"[{"type":"https://pdpp.dev/data-access"}]"#,
            &resolved(&json!([access(&json!({}))]))
                .unwrap()
                .to_document()
                .unwrap()
                .replace("training_coach", "clinician_summary"),
            &resolved(&json!([access(&json!({}))]))
                .unwrap()
                .to_document()
                .unwrap()
                .replace("P30D", "P1M"),
        ] {
            assert!(
                ResolvedDetails::from_document(document).is_err(),
                "bug: {document} read"
            );
        }
    }

    #[test]
    fn a_plain_grants_digest_is_its_text_version() {
        assert_eq!(
            review_digest("text-v1", None).unwrap(),
            "text-v1",
            "bug: a plain grant's digest is not its text version"
        );
    }

    #[test]
    fn the_digest_binds_the_terms_and_not_the_frozen_window() {
        let training = resolved(&json!([access(&json!({}))])).unwrap();
        let sleep = resolved(&json!([access(
            &json!({"selection_preset": "sleep_coach"})
        )]))
        .unwrap();
        let digest = review_digest("text-v1", Some(&training)).unwrap();
        assert_ne!(
            digest, "text-v1",
            "bug: a detailed grant's digest is its bare text version"
        );
        assert_ne!(
            digest,
            review_digest("text-v2", Some(&training)).unwrap(),
            "bug: the digest does not bind the text version"
        );
        assert_ne!(
            digest,
            review_digest("text-v1", Some(&sleep)).unwrap(),
            "bug: the digest does not bind the preset"
        );
        let (frozen, _) = training.freeze(Timestamp::now(), &Zone::utc()).unwrap();
        assert_eq!(
            digest,
            review_digest("text-v1", Some(&frozen)).unwrap(),
            "bug: freezing the window changed the digest"
        );
    }

    #[test]
    fn the_picker_defaults_are_what_the_preset_alone_resolves_to() {
        for preset in Preset::ALL
            .into_iter()
            .filter(|preset| !preset.requires_step_up())
        {
            let asked = resolved(&json!([access(
                &json!({"selection_preset": preset.name()})
            )]))
            .unwrap();
            assert_eq!(
                preset_defaults(preset, MCP),
                asked,
                "bug: the picker grants other terms than the preset alone resolves to"
            );
        }
    }
}
