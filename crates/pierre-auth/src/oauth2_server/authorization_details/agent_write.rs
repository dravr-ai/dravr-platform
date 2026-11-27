// ABOUTME: The agent-write detail (owl PDPP §4.5): which typed records a client may propose, checked against barrage's row schemas
// ABOUTME: Parsed before sign-in, stored as the grant's document, echoed by introspection; a proposal is never read until the athlete confirms it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent-write grants (carnet#1544 slice 4c, owl PDPP §4.5)
//!
//! A client asks to **propose** records with one
//! [`AGENT_WRITE_TYPE`] detail: up to ten `targets`, each a writable stream
//! and the fields of its row schema the client may send, a daily record
//! cap, a lifetime of at most 90 days, and how its tokens are bound
//! (`sender_constraint`):
//! - `dpop`: a profile grant of its own, minting `agent_write` tokens for the
//!   agent-write resource (binding A);
//! - `none`: a child grant inside the client's MCP package, used by the MCP
//!   write tool under the host's own token (binding B).
//!
//! Every stream, field and requirement is barrage's
//! ([`dravr_barrage::agent_write::row_schema`]): a field the server sets is
//! refused by name, so is a stream no record of which can be proposed, and a
//! target must name its stream's required fields.

use std::collections::BTreeSet;

use dravr_barrage::agent_write::row_schema;
use dravr_barrage::{Stream, Tier};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::duration::SECONDS_PER_DAY;
use super::{refused, STREAM_NAME_PREFIX};
use crate::oauth2_server::models::OAuth2Error;

/// Dravr's agent-write detail type. Owl's `health-profile.example` URIs are
/// placeholders (PDPP §14, open issue 1); this is Dravr's namespace, as the
/// health-constraints type is.
pub const AGENT_WRITE_TYPE: &str = "https://dravr.ai/pdpp/health-profile/v0/agent-write";

pub use pierre_core::models::AGENT_WRITE_SCOPE;

/// Whether a stored grant, code or refresh `scope` is an agent-write
/// grant's.
#[must_use]
pub fn is_agent_write_scope(scope: Option<&str>) -> bool {
    scope == Some(AGENT_WRITE_SCOPE)
}

/// The path of the agent-write resource on the platform's origin: its
/// identifier, and the base of `POST /records` (PDPP §5.3).
pub const AGENT_WRITE_RESOURCE_PATH: &str = "/api/agent-write/v0";

/// The records a day, and the lifetime, of a detail the authorization server
/// builds from a `health:write:<stream>` step-up scope. Owl names no default;
/// the athlete sees both on the consent screen.
pub const WRITE_STEP_UP_RECORDS_PER_DAY: u32 = 50;
const WRITE_STEP_UP_LIFETIME: &str = "P30D";

/// The most targets one detail names (PDPP §4.5 schema `maxItems`).
const MAX_TARGETS: usize = 10;
/// The most records a day a grant may propose (PDPP §4.5 schema).
const MAX_RECORDS_PER_DAY: u32 = 500;
/// The longest an agent-write grant lasts, in days (PDPP §4.5 `lifetime`).
const MAX_LIFETIME_DAYS: i64 = 90;

/// How an agent-write grant's tokens are bound (PDPP §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderConstraint {
    /// A profile grant of its own: `agent_write` tokens, DPoP-bound
    Dpop,
    /// A child inside the MCP package: the host's own token, which the MCP
    /// audience bounds (PDPP §7.3)
    None,
}

/// A grant's record caps (PDPP §4.5 `rate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWriteRate {
    /// Records proposed per UTC day, 1 to 500
    pub records_per_day: u32,
    /// Requests per hour; `None` is no hourly cap, the daily cap bounding
    /// the grant
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_per_hour: Option<u32>,
}

/// The fresh sign-in an agent-write approval stood on (PDPP §4.2
/// `step_up_result`): set by the authorization server, never by a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepUpResult {
    /// When the athlete signed in on Dravr's hosted page
    pub satisfied_at: Timestamp,
    /// The assurance the sign-in reached
    pub acr: String,
}

/// One stream a grant may propose into, and the fields it may send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentWriteTarget {
    /// The writable stream
    pub stream: Stream,
    /// The proposable fields granted, in the order the client named them
    pub fields: Vec<&'static str>,
}

/// The wire form of a target: owl's stream name with its prefix.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetWire {
    stream: String,
    fields: Vec<String>,
}

/// The wire form of the detail, as a client sends it and a grant stores it
/// (the `type` read apart).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailWire {
    critical: bool,
    token_kind: String,
    targets: Vec<TargetWire>,
    max_tier: Tier,
    rate: AgentWriteRate,
    lifetime: String,
    review: String,
    sender_constraint: SenderConstraint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    step_up_result: Option<StepUpResult>,
}

/// An agent-write detail whose every value passed its checks: what the
/// consent screen shows, the grant stores and introspection echoes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentWrite {
    targets: Vec<AgentWriteTarget>,
    max_tier: Tier,
    rate: AgentWriteRate,
    lifetime: String,
    lifetime_seconds: i64,
    sender_constraint: SenderConstraint,
    step_up_result: Option<StepUpResult>,
}

impl ResolvedAgentWrite {
    /// The detail at `index` of a request, its `type` already read.
    ///
    /// # Errors
    /// `invalid_authorization_details` for an unknown member or any value
    /// PDPP §4.5 or the target streams' row schemas refuse.
    pub(super) fn parse(index: usize, members: Map<String, Value>) -> Result<Self, OAuth2Error> {
        let wire: DetailWire = serde_json::from_value(Value::Object(members))
            .map_err(|e| refused(&format!("authorization_details[{index}]: {e}")))?;
        Self::checked(wire)
    }

    /// The detail a `health:write:<stream>` step-up asks for: every
    /// proposable field of each stream, bound by the MCP package
    /// (`sender_constraint: none`), [`WRITE_STEP_UP_RECORDS_PER_DAY`] a day
    /// for 30 days.
    ///
    /// # Errors
    /// `invalid_scope` for a stream no record of which can be proposed.
    pub fn for_write_scope(streams: &[Stream]) -> Result<Self, OAuth2Error> {
        let mut targets = Vec::with_capacity(streams.len());
        for stream in streams {
            let schema = row_schema(*stream).ok_or_else(|| {
                OAuth2Error::invalid_scope(&format!(
                    "no record of {} can be proposed",
                    stream.name()
                ))
            })?;
            targets.push(TargetWire {
                stream: format!("{STREAM_NAME_PREFIX}{}", stream.name()),
                fields: schema.fields.iter().map(|f| f.name.to_owned()).collect(),
            });
        }
        let max_tier = streams
            .iter()
            .map(|stream| stream.tier())
            .max()
            .unwrap_or(Tier::T0);
        Self::checked(DetailWire {
            critical: true,
            token_kind: "agent_write".to_owned(),
            targets,
            max_tier,
            rate: AgentWriteRate {
                records_per_day: WRITE_STEP_UP_RECORDS_PER_DAY,
                requests_per_hour: None,
            },
            lifetime: WRITE_STEP_UP_LIFETIME.to_owned(),
            review: "owner_confirm".to_owned(),
            sender_constraint: SenderConstraint::None,
            step_up_result: None,
        })
    }

    /// `wire`'s values checked against PDPP §4.5 and the row schemas.
    fn checked(wire: DetailWire) -> Result<Self, OAuth2Error> {
        if !wire.critical {
            return Err(refused("an agent-write detail is critical: true"));
        }
        if wire.token_kind != "agent_write" {
            return Err(refused("an agent-write detail's token_kind is agent_write"));
        }
        if wire.review != "owner_confirm" {
            return Err(refused(
                "an agent-write detail's review is owner_confirm: every write is pending",
            ));
        }
        if wire.targets.is_empty() || wire.targets.len() > MAX_TARGETS {
            return Err(refused(&format!(
                "an agent-write detail names 1 to {MAX_TARGETS} targets"
            )));
        }
        let mut targets: Vec<AgentWriteTarget> = Vec::with_capacity(wire.targets.len());
        for target in &wire.targets {
            let checked = check_target(target)?;
            if targets.iter().any(|seen| seen.stream == checked.stream) {
                return Err(refused(&format!(
                    "{} is named by two targets",
                    target.stream
                )));
            }
            targets.push(checked);
        }
        let highest = targets
            .iter()
            .map(|target| target.stream.tier())
            .max()
            .unwrap_or(Tier::T0);
        if wire.max_tier != highest {
            return Err(refused(&format!(
                "max_tier must be {highest:?}, the highest tier of the targets"
            )));
        }
        if !(1..=MAX_RECORDS_PER_DAY).contains(&wire.rate.records_per_day) {
            return Err(refused(&format!(
                "rate.records_per_day must be 1 to {MAX_RECORDS_PER_DAY}"
            )));
        }
        if wire.rate.requests_per_hour == Some(0) {
            return Err(refused("rate.requests_per_hour must be at least 1"));
        }
        let lifetime_seconds = lifetime_seconds(&wire.lifetime).ok_or_else(|| {
            refused(&format!(
                "lifetime must be P<n>D or P<n>W, at most P{MAX_LIFETIME_DAYS}D"
            ))
        })?;
        Ok(Self {
            targets,
            max_tier: wire.max_tier,
            rate: wire.rate,
            lifetime: wire.lifetime,
            lifetime_seconds,
            sender_constraint: wire.sender_constraint,
            step_up_result: wire.step_up_result,
        })
    }

    /// The streams the grant may propose into, with their granted fields.
    #[must_use]
    pub fn targets(&self) -> &[AgentWriteTarget] {
        &self.targets
    }

    /// The target for `stream`, when the grant names it.
    #[must_use]
    pub fn target(&self, stream: Stream) -> Option<&AgentWriteTarget> {
        self.targets.iter().find(|target| target.stream == stream)
    }

    /// The highest tier of the targets.
    #[must_use]
    pub const fn max_tier(&self) -> Tier {
        self.max_tier
    }

    /// The grant's record caps.
    #[must_use]
    pub const fn rate(&self) -> AgentWriteRate {
        self.rate
    }

    /// How the grant's tokens are bound.
    #[must_use]
    pub const fn sender_constraint(&self) -> SenderConstraint {
        self.sender_constraint
    }

    /// How long the grant lasts from approval.
    #[must_use]
    pub const fn lifetime(&self) -> Elapsed {
        Elapsed::seconds(self.lifetime_seconds)
    }

    /// The fresh sign-in the approval stood on; `None` before approval.
    #[must_use]
    pub const fn step_up_result(&self) -> Option<&StepUpResult> {
        self.step_up_result.as_ref()
    }

    /// This detail approved at `now` after the fresh sign-in `result`: the
    /// stored document, and when the grant expires.
    ///
    /// # Errors
    /// An internal error when the expiry falls outside the calendar.
    pub fn approve(&self, now: Timestamp, result: StepUpResult) -> AppResult<(Self, Timestamp)> {
        let expires_at = now.checked_add(self.lifetime()).ok_or_else(|| {
            AppError::internal("An agent-write grant's expiry falls outside the calendar")
        })?;
        let mut approved = self.clone();
        approved.step_up_result = Some(result);
        Ok((approved, expires_at))
    }

    /// The detail's terms, without the approval's sign-in: what the athlete
    /// reviews, and what two grants are compared by.
    #[must_use]
    pub fn terms(&self) -> Self {
        let mut terms = self.clone();
        terms.step_up_result = None;
        terms
    }

    /// Whether `other` grants exactly what this detail grants.
    #[must_use]
    pub fn same_terms(&self, other: &Self) -> bool {
        self.terms() == other.terms()
    }

    /// The detail as RFC 9396 sends it: one object, `type` first.
    fn to_value(&self) -> AppResult<Value> {
        let wire = DetailWire {
            critical: true,
            token_kind: "agent_write".to_owned(),
            targets: self
                .targets
                .iter()
                .map(|target| TargetWire {
                    stream: format!("{STREAM_NAME_PREFIX}{}", target.stream.name()),
                    fields: target.fields.iter().map(|f| (*f).to_owned()).collect(),
                })
                .collect(),
            max_tier: self.max_tier,
            rate: self.rate,
            lifetime: self.lifetime.clone(),
            review: "owner_confirm".to_owned(),
            sender_constraint: self.sender_constraint,
            step_up_result: self.step_up_result.clone(),
        };
        let Value::Object(members) = serde_json::to_value(&wire).map_err(|e| {
            AppError::internal(format!("Failed to serialize an agent-write detail: {e}"))
        })?
        else {
            return Err(AppError::internal("An agent-write detail is not an object"));
        };
        let mut detail = Map::new();
        detail.insert(
            "type".to_owned(),
            Value::String(AGENT_WRITE_TYPE.to_owned()),
        );
        detail.extend(members);
        Ok(Value::Object(detail))
    }

    /// The stored document: an RFC 9396 array of this one detail.
    ///
    /// # Errors
    /// An internal error if the detail cannot be serialized.
    pub fn to_document(&self) -> AppResult<String> {
        Ok(Value::Array(vec![self.to_value()?]).to_string())
    }

    /// The detail a grant stored.
    ///
    /// # Errors
    /// An internal error when `document` is not an agent-write document this
    /// server wrote, or no longer passes its checks.
    pub fn from_document(document: &str) -> AppResult<Self> {
        let unreadable = |why: &str| {
            AppError::internal(format!("A grant's agent-write detail does not read: {why}"))
        };
        let details: Vec<Map<String, Value>> =
            serde_json::from_str(document).map_err(|e| unreadable(&e.to_string()))?;
        let Ok([mut detail]) = <[_; 1]>::try_from(details) else {
            return Err(unreadable("it is not one detail"));
        };
        match detail.remove("type") {
            Some(Value::String(kind)) if kind == AGENT_WRITE_TYPE => {}
            _ => return Err(unreadable("its type is not agent-write")),
        }
        let wire: DetailWire = serde_json::from_value(Value::Object(detail))
            .map_err(|e| unreadable(&e.to_string()))?;
        Self::checked(wire).map_err(|e| unreadable(&e.error_description.unwrap_or_default()))
    }
}

/// One target checked: a writable stream, its fields proposable and named
/// once each, its required fields among them.
fn check_target(target: &TargetWire) -> Result<AgentWriteTarget, OAuth2Error> {
    let name = target
        .stream
        .strip_prefix(STREAM_NAME_PREFIX)
        .filter(|name| name.ends_with(".summary"))
        .ok_or_else(|| {
            refused(&format!(
                "{}: an agent-write target is a {STREAM_NAME_PREFIX}<name>.summary stream",
                target.stream
            ))
        })?;
    let schema = Stream::from_name(name)
        .and_then(row_schema)
        .ok_or_else(|| {
            refused(&format!(
                "{}: no record of that stream can be proposed",
                target.stream
            ))
        })?;
    if target.fields.is_empty() {
        return Err(refused(&format!(
            "{}: fields names no field",
            target.stream
        )));
    }
    let mut seen = BTreeSet::new();
    let mut fields = Vec::with_capacity(target.fields.len());
    for field in &target.fields {
        if !seen.insert(field.as_str()) {
            return Err(refused(&format!(
                "{}: {field} is named twice",
                target.stream
            )));
        }
        if schema.is_as_set(field) {
            return Err(refused(&format!(
                "{}: {field} is set by the server, never proposed",
                target.stream
            )));
        }
        let spec = schema.field(field).ok_or_else(|| {
            refused(&format!(
                "{}: {field} is not a field of that stream",
                target.stream
            ))
        })?;
        fields.push(spec.name);
    }
    if let Some(missing) = schema
        .required()
        .find(|required| !fields.contains(required))
    {
        return Err(refused(&format!(
            "{}: fields must include {missing}, which every record carries",
            target.stream
        )));
    }
    Ok(AgentWriteTarget {
        stream: schema.stream,
        fields,
    })
}

/// `text` as PDPP §4.5 spells a lifetime (`P<n>D` or `P<n>W`), in seconds,
/// when it is at most [`MAX_LIFETIME_DAYS`].
fn lifetime_seconds(text: &str) -> Option<i64> {
    let rest = text.strip_prefix('P')?;
    let (count, days_per_unit) = rest
        .strip_suffix('D')
        .map(|count| (count, 1))
        .or_else(|| rest.strip_suffix('W').map(|count| (count, 7)))?;
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let days = count.parse::<i64>().ok()?.checked_mul(days_per_unit)?;
    (1..=MAX_LIFETIME_DAYS)
        .contains(&days)
        .then(|| days * SECONDS_PER_DAY)
}

/// The review digest an agent-write approval is bound to (owl §3.3): the
/// SHA-256 of the consent text version and the detail's terms, so the
/// targets, rate and end a screen showed are what is approved.
///
/// # Errors
/// An internal error if the detail cannot be serialized.
pub fn agent_write_review_digest(
    text_version: &str,
    detail: &ResolvedAgentWrite,
) -> AppResult<String> {
    let mut digest = Sha256::new();
    digest.update(text_version.as_bytes());
    digest.update(b"\nagent-write\n");
    digest.update(detail.terms().to_document()?.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_barrage::agent_write::writable_streams;
    use serde_json::json;

    /// An agent-write detail, `extra`'s members laid over a valid one.
    pub(crate) fn detail(extra: &Value) -> Map<String, Value> {
        let mut detail = json!({
            "critical": true,
            "token_kind": "agent_write",
            "targets": [{
                "stream": "health.vault.recovery.daily.summary",
                "fields": ["local_date", "soreness", "fatigue"],
            }],
            "max_tier": "T1",
            "rate": {"records_per_day": 5},
            "lifetime": "P30D",
            "review": "owner_confirm",
            "sender_constraint": "dpop",
        });
        if let (Value::Object(detail), Value::Object(extra)) = (&mut detail, extra) {
            detail.extend(extra.clone());
        }
        let Value::Object(detail) = detail else {
            unreachable!("a literal object")
        };
        detail
    }

    fn parsed(extra: &Value) -> Result<ResolvedAgentWrite, String> {
        ResolvedAgentWrite::parse(0, detail(extra))
            .map_err(|e| e.error_description.unwrap_or_default())
    }

    #[test]
    fn owls_example_parses_with_a_subset_of_fields() {
        let write = parsed(&Value::Null).expect("owl's example");
        assert_eq!(write.targets().len(), 1);
        assert_eq!(write.targets()[0].stream, Stream::RecoveryDaily);
        assert_eq!(
            write.targets()[0].fields,
            vec!["local_date", "soreness", "fatigue"]
        );
        assert_eq!(write.max_tier(), Tier::T1);
        assert_eq!(write.rate().records_per_day, 5);
        assert_eq!(write.rate().requests_per_hour, None);
        assert_eq!(write.sender_constraint(), SenderConstraint::Dpop);
        assert_eq!(write.lifetime(), Elapsed::seconds(30 * SECONDS_PER_DAY));
    }

    #[test]
    fn each_field_rule_refuses() {
        let cases = [
            (json!({"critical": false}), "critical"),
            (json!({"token_kind": "client"}), "token_kind"),
            (json!({"review": "auto"}), "owner_confirm"),
            (json!({"targets": []}), "1 to 10 targets"),
            (json!({"max_tier": "T2"}), "max_tier must be T1"),
            (json!({"max_tier": "T0"}), "max_tier must be T1"),
            (json!({"rate": {"records_per_day": 0}}), "records_per_day"),
            (json!({"rate": {"records_per_day": 501}}), "records_per_day"),
            (json!({"rate": {}}), "records_per_day"),
            (
                json!({"rate": {"records_per_day": 5, "requests_per_hour": 0}}),
                "requests_per_hour",
            ),
            (json!({"lifetime": "P91D"}), "lifetime"),
            (json!({"lifetime": "P13W"}), "lifetime"),
            (json!({"lifetime": "PT1H"}), "lifetime"),
            (json!({"lifetime": "P0D"}), "lifetime"),
            (json!({"sender_constraint": "mtls"}), "mtls"),
            (json!({"surprise": 1}), "surprise"),
            (
                json!({"targets": [{"stream": "health.vault.recovery.daily.summary",
                    "fields": ["local_date", "soreness", "soreness"]}]}),
                "named twice",
            ),
            (
                json!({"targets": [{"stream": "health.vault.recovery.daily.summary",
                    "fields": []}]}),
                "names no field",
            ),
            (
                json!({"targets": [{"stream": "health.vault.recovery.daily.summary",
                    "fields": ["soreness"]}]}),
                "must include local_date",
            ),
            (
                json!({"targets": [{"stream": "health.vault.recovery.daily.summary",
                    "fields": ["local_date", "notes"]}]}),
                "not a field",
            ),
        ];
        for (extra, needle) in cases {
            let error = parsed(&extra).expect_err(&format!("bug: {extra} was accepted"));
            assert!(error.contains(needle), "bug: {extra} refused as {error:?}");
        }
        let eleven: Vec<Value> = (0..11)
            .map(|_| json!({"stream": "health.vault.recovery.daily.summary", "fields": ["local_date"]}))
            .collect();
        assert!(parsed(&json!({"targets": eleven}))
            .unwrap_err()
            .contains("1 to 10 targets"));
    }

    #[test]
    fn a_field_the_server_sets_is_refused_by_name() {
        for (stream, field, tier) in [
            ("health.vault.activity.daily.summary", "selected_from", "T0"),
            ("health.vault.labs.summary", "display", "T2"),
            ("health.vault.labs.summary", "report_id", "T2"),
            ("health.vault.recovery.daily.summary", "source_class", "T1"),
            ("health.vault.workout.summary", "id", "T0"),
        ] {
            let error = parsed(&json!({
                "targets": [{"stream": stream, "fields": [field]}],
                "max_tier": tier,
            }))
            .expect_err(&format!("bug: AS-set {field} of {stream} accepted"));
            assert!(
                error.contains(&format!("{field} is set by the server")),
                "bug: {field} refused as {error:?}"
            );
        }
    }

    #[test]
    fn every_excluded_stream_is_refused_by_name() {
        for stream in Stream::ALL {
            let name = format!("{STREAM_NAME_PREFIX}{}", stream.name());
            let writable = writable_streams().any(|w| w == stream);
            let schema_fields = row_schema(stream).map(|schema| {
                schema
                    .required()
                    .chain(schema.fields.first().map(|f| f.name))
                    .collect::<BTreeSet<_>>()
            });
            let fields: Vec<&str> = schema_fields
                .map(|f| f.into_iter().collect())
                .unwrap_or_else(|| vec!["value"]);
            let result = parsed(&json!({
                "targets": [{"stream": name, "fields": fields}],
                "max_tier": stream.tier(),
            }));
            if writable {
                assert!(result.is_ok(), "bug: writable {name} refused: {result:?}");
            } else {
                let error = result.expect_err(&format!("bug: {name} accepted as a target"));
                assert!(
                    error.contains(&name),
                    "bug: the refusal of {name} does not name it: {error:?}"
                );
            }
        }
        for name in [
            "health.vault.agent.pending",
            "recovery.daily.summary",
            "health.vault.nope.summary",
        ] {
            let error = parsed(&json!({"targets": [{"stream": name, "fields": ["x"]}]}))
                .expect_err(&format!("bug: {name} accepted"));
            assert!(error.contains(name), "bug: {name} refused as {error:?}");
        }
    }

    #[test]
    fn seventeen_streams_are_writable_and_none_is_t3() {
        let streams: Vec<Stream> = writable_streams().collect();
        assert_eq!(streams.len(), 17, "bug: owl writes 17 streams: {streams:?}");
        assert!(streams.iter().all(|s| s.tier() <= Tier::T2));
    }

    #[test]
    fn the_document_round_trips_and_terms_ignore_the_sign_in() {
        let write =
            parsed(&json!({"rate": {"records_per_day": 5, "requests_per_hour": 3}})).unwrap();
        let now = Timestamp::from_epoch_seconds(1_800_000_000).unwrap();
        let (approved, expires_at) = write
            .approve(
                now,
                StepUpResult {
                    satisfied_at: now,
                    acr: "owner_verified".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(
            expires_at.epoch_seconds(),
            1_800_000_000 + 30 * SECONDS_PER_DAY
        );
        let document = approved.to_document().unwrap();
        let read = ResolvedAgentWrite::from_document(&document).unwrap();
        assert_eq!(read, approved);
        assert!(read.same_terms(&write));
        assert_eq!(
            read.step_up_result().map(|r| r.acr.as_str()),
            Some("owner_verified")
        );
        let value: Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value[0]["type"], AGENT_WRITE_TYPE);
        assert_eq!(
            value[0]["targets"][0]["stream"],
            "health.vault.recovery.daily.summary"
        );
        assert!(ResolvedAgentWrite::from_document("[]").is_err());
        assert!(ResolvedAgentWrite::from_document(
            &json!([{"type": "https://pdpp.dev/data-access"}]).to_string()
        )
        .is_err());
    }

    #[test]
    fn the_digest_moves_with_the_terms_and_not_the_sign_in() {
        let write = parsed(&Value::Null).unwrap();
        let wider = parsed(&json!({"rate": {"records_per_day": 6}})).unwrap();
        let now = Timestamp::from_epoch_seconds(1_800_000_000).unwrap();
        let (approved, _) = write
            .approve(
                now,
                StepUpResult {
                    satisfied_at: now,
                    acr: "owner_verified".to_owned(),
                },
            )
            .unwrap();
        let digest = agent_write_review_digest("v1", &write).unwrap();
        assert_eq!(digest, agent_write_review_digest("v1", &approved).unwrap());
        assert_ne!(digest, agent_write_review_digest("v1", &wider).unwrap());
        assert_ne!(digest, agent_write_review_digest("v2", &write).unwrap());
    }

    #[test]
    fn a_write_step_up_asks_every_field_under_the_package() {
        let write = ResolvedAgentWrite::for_write_scope(&[Stream::Labs, Stream::Sleep]).unwrap();
        assert_eq!(write.sender_constraint(), SenderConstraint::None);
        assert_eq!(write.max_tier(), Tier::T2);
        assert_eq!(write.rate().records_per_day, WRITE_STEP_UP_RECORDS_PER_DAY);
        assert_eq!(write.lifetime(), Elapsed::seconds(30 * SECONDS_PER_DAY));
        let labs = write.target(Stream::Labs).unwrap();
        let schema = row_schema(Stream::Labs).unwrap();
        assert_eq!(labs.fields.len(), schema.fields.len());
        assert!(ResolvedAgentWrite::for_write_scope(&[Stream::WorkoutRaw]).is_err());
    }
}
