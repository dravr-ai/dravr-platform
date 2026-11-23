// ABOUTME: A client's authorization_details parsed and checked against the field table, before the athlete signs in
// ABOUTME: Every member the server cannot enforce, every AS-set and every unknown member is refused here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The request side: [`parse_authorization_details`] reads the parameter a
//! client sends into [`RequestedDetails`], refusing whatever this server does
//! not resolve, so [`super::resolve`] only ever narrows checked values.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::duration::parse_seconds;
use super::{
    refused, ClientClaims, Preset, Representation, Retention, Source, Tier, DATA_ACCESS_TYPE,
    HEALTH_CONSTRAINTS_TYPE, MAX_AUTHORIZATION_DETAILS_BYTES,
};
use crate::oauth2_server::models::OAuth2Error;

/// The longest purpose a client may describe itself with, in characters: it
/// is quoted on the consent screen.
const MAX_PURPOSE_CHARS: usize = 500;

/// The most commitments a client may list, and the longest each may be.
const MAX_COMMITMENTS: usize = 10;
const MAX_COMMITMENT_CHARS: usize = 300;

/// The longest identifier-like value read (a purpose code, a DPV IRI, a
/// retention term), in characters.
const MAX_TOKEN_CHARS: usize = 256;

/// The longest `source.id` read: a resource URL.
const MAX_SOURCE_CHARS: usize = 2_048;

/// Members only the authorization server sets (owl §4.2 "AS-set").
const AS_SET_MEMBERS: [&str; 3] = ["host_profile", "standing_policy_ref", "step_up_result"];

/// Members of a data-access detail this server cannot enforce yet, and why.
const REFUSED_ACCESS_MEMBERS: [(&str, &str); 2] = [
    (
        "streams",
        "explicit streams are not granted; ask for a selection_preset",
    ),
    (
        "time_range",
        "time_range is not granted; a preset's data window is set by the server",
    ),
];

/// Members of a health-constraints detail this server cannot enforce yet,
/// and why.
const REFUSED_CONSTRAINT_MEMBERS: [(&str, &str); 3] = [
    (
        "raw_streams",
        "raw_streams are not granted; representation chooses the preset's raw streams",
    ),
    (
        "code_filter",
        "code_filter is not granted; no stream a preset names is code-filterable",
    ),
    ("egress", "egress is not enforced yet"),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DataAccessRequest {
    pub(super) source: Option<Source>,
    selection_preset: Option<String>,
    access_mode: Option<String>,
    pub(super) purpose_code: Option<String>,
    pub(super) purpose_description: Option<String>,
    pub(super) client_claims: Option<ClientClaims>,
    pub(super) retention: Option<Retention>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintsRequest {
    applies_to: Option<u64>,
    critical: Option<bool>,
    max_tier: Option<Tier>,
    representation: Option<Representation>,
    lifetime: Option<String>,
    purpose_dpv: Option<String>,
    budgets: Option<BudgetsRequest>,
    step_up: Option<StepUpRequest>,
    host_profile_required: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BudgetsRequest {
    pub(super) requests_per_hour: Option<u32>,
    pub(super) rows_per_day: Option<u32>,
    pub(super) raw_rows_per_day: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepUpRequest {
    required: bool,
    acr: Option<String>,
}

/// A request's `authorization_details`, parsed and checked but not yet
/// resolved against the resource the tokens are for.
#[derive(Debug)]
pub struct RequestedDetails {
    pub(super) access: DataAccessRequest,
    pub(super) preset: Preset,
    pub(super) constraints: Option<CheckedConstraints>,
}

/// A health-constraints detail whose values passed their checks.
#[derive(Debug)]
pub(super) struct CheckedConstraints {
    pub(super) max_tier: Option<Tier>,
    pub(super) representation: Option<Representation>,
    pub(super) lifetime_seconds: Option<i64>,
    pub(super) purpose_dpv: Option<String>,
    pub(super) budgets: Option<BudgetsRequest>,
}

/// Parse an `authorization_details` parameter (RFC 9396 §2): a JSON array
/// of exactly one data-access detail naming a preset and at most one
/// health-constraints detail applying to it.
///
/// # Errors
/// `invalid_authorization_details` for anything this server does not
/// resolve: a malformed or oversized value, an unknown type or member, an
/// AS-set member, a value it cannot enforce, or a detail the profile's rules
/// refuse.
pub fn parse_authorization_details(raw: &str) -> Result<RequestedDetails, OAuth2Error> {
    if raw.len() > MAX_AUTHORIZATION_DETAILS_BYTES {
        return Err(refused(&format!(
            "authorization_details must not exceed {MAX_AUTHORIZATION_DETAILS_BYTES} bytes"
        )));
    }
    let details: Vec<Value> = serde_json::from_str(raw)
        .map_err(|_| refused("authorization_details must be a JSON array of objects"))?;
    if details.is_empty() {
        return Err(refused("authorization_details names no detail"));
    }
    let mut accesses = Vec::new();
    let mut constraints = Vec::new();
    for (index, detail) in details.into_iter().enumerate() {
        let Value::Object(mut members) = detail else {
            return Err(refused(&format!(
                "authorization_details[{index}] is not an object"
            )));
        };
        let Some(Value::String(kind)) = members.remove("type") else {
            return Err(refused(&format!(
                "authorization_details[{index}] has no type"
            )));
        };
        refuse_members(index, &members, &AS_SET_MEMBERS.map(|m| (m, "")), true)?;
        match kind.as_str() {
            DATA_ACCESS_TYPE => {
                refuse_members(index, &members, &REFUSED_ACCESS_MEMBERS, false)?;
                accesses.push((index, typed::<DataAccessRequest>(index, members)?));
            }
            HEALTH_CONSTRAINTS_TYPE => {
                refuse_members(index, &members, &REFUSED_CONSTRAINT_MEMBERS, false)?;
                refuse_total_rows(index, &members)?;
                constraints.push(typed::<ConstraintsRequest>(index, members)?);
            }
            other => {
                return Err(refused(&format!(
                    "authorization_details[{index}] has type {other}, which this server does not support"
                )));
            }
        }
    }
    let Ok([(access_index, access)]) = <[_; 1]>::try_from(accesses) else {
        return Err(refused(
            "authorization_details must carry exactly one https://pdpp.dev/data-access detail",
        ));
    };
    if constraints.len() > 1 {
        return Err(refused(
            "at most one health-constraints detail may apply to a data-access detail",
        ));
    }
    let preset = check_access(&access)?;
    let constraints = constraints
        .pop()
        .map(|constraint| check_constraints(constraint, access_index))
        .transpose()?;
    Ok(RequestedDetails {
        access,
        preset,
        constraints,
    })
}

/// Refuse any of `refusals`' members `members` carries. An AS-set member
/// says so; the others give their own reason.
fn refuse_members(
    index: usize,
    members: &Map<String, Value>,
    refusals: &[(&str, &str)],
    as_set: bool,
) -> Result<(), OAuth2Error> {
    for (member, reason) in refusals {
        if members.contains_key(*member) {
            let why = if as_set {
                format!("{member} is set by the authorization server, never by a client")
            } else {
                (*reason).to_owned()
            };
            return Err(refused(&format!(
                "authorization_details[{index}].{member}: {why}"
            )));
        }
    }
    Ok(())
}

/// Refuse a `total_rows` budget: it belongs to single-use grants.
fn refuse_total_rows(index: usize, members: &Map<String, Value>) -> Result<(), OAuth2Error> {
    let total_rows = members
        .get("budgets")
        .and_then(Value::as_object)
        .is_some_and(|budgets| budgets.contains_key("total_rows"));
    if total_rows {
        return Err(refused(&format!(
            "authorization_details[{index}].budgets.total_rows belongs to single-use grants, which are not granted yet"
        )));
    }
    Ok(())
}

/// `members` read as `T`, every unknown member refused.
fn typed<T: DeserializeOwned>(index: usize, members: Map<String, Value>) -> Result<T, OAuth2Error> {
    serde_json::from_value(Value::Object(members))
        .map_err(|e| refused(&format!("authorization_details[{index}]: {e}")))
}

/// Whether `text` is at most `max` characters with no control character.
fn plain_text(text: &str, max: usize) -> bool {
    text.chars().count() <= max && !text.chars().any(char::is_control)
}

/// Check a data-access detail's values; the preset it names.
fn check_access(access: &DataAccessRequest) -> Result<Preset, OAuth2Error> {
    let source = access
        .source
        .as_ref()
        .ok_or_else(|| refused("the data-access detail must name its source"))?;
    if source.id.is_empty() || !plain_text(&source.id, MAX_SOURCE_CHARS) {
        return Err(refused("source.id must be the resource the tokens are for"));
    }
    let preset = access
        .selection_preset
        .as_deref()
        .ok_or_else(|| refused("the data-access detail must name a selection_preset"))?;
    let preset = Preset::from_name(preset)
        .ok_or_else(|| refused("selection_preset must be training_coach or sleep_coach"))?;
    match access.access_mode.as_deref() {
        None | Some("continuous") => {}
        Some("single_use") => {
            return Err(refused("single_use grants are not granted yet"));
        }
        Some(_) => return Err(refused("access_mode must be continuous")),
    }
    if access
        .purpose_code
        .as_deref()
        .is_some_and(|code| !plain_text(code, MAX_TOKEN_CHARS))
    {
        return Err(refused(
            "purpose_code is too long or carries a control character",
        ));
    }
    if access
        .purpose_description
        .as_deref()
        .is_some_and(|purpose| !plain_text(purpose, MAX_PURPOSE_CHARS))
    {
        return Err(refused(&format!(
            "purpose_description must be at most {MAX_PURPOSE_CHARS} characters of plain text"
        )));
    }
    if let Some(claims) = &access.client_claims {
        let too_many = claims.commitments.len() > MAX_COMMITMENTS;
        if too_many
            || claims
                .commitments
                .iter()
                .any(|commitment| !plain_text(commitment, MAX_COMMITMENT_CHARS))
        {
            return Err(refused(&format!(
                "client_claims.commitments must be at most {MAX_COMMITMENTS} lines of at most {MAX_COMMITMENT_CHARS} characters"
            )));
        }
    }
    if let Some(retention) = &access.retention {
        let duration = retention.max_duration.starts_with('P')
            && retention.max_duration.len() <= MAX_TOKEN_CHARS
            && retention
                .max_duration
                .chars()
                .all(|c| c.is_ascii_alphanumeric());
        if !duration || !plain_text(&retention.on_expiry, MAX_TOKEN_CHARS) {
            return Err(refused(
                "retention must carry an ISO 8601 max_duration and an on_expiry term",
            ));
        }
    }
    Ok(preset)
}

/// Check a health-constraints detail applying to the data-access detail at
/// `access_index`.
fn check_constraints(
    constraint: ConstraintsRequest,
    access_index: usize,
) -> Result<CheckedConstraints, OAuth2Error> {
    if constraint.critical != Some(true) {
        return Err(refused(
            "a health-constraints detail must carry critical: true",
        ));
    }
    if constraint.applies_to != Some(access_index as u64) {
        return Err(refused(
            "applies_to must be the index of the data-access detail it narrows",
        ));
    }
    if matches!(constraint.max_tier, Some(Tier::T2 | Tier::T3)) {
        return Err(refused("max_tier above T1 is not granted yet"));
    }
    let lifetime_seconds = constraint
        .lifetime
        .as_deref()
        .map(|lifetime| {
            parse_seconds(lifetime).ok_or_else(|| {
                refused("lifetime must be an ISO 8601 duration in weeks, days, hours, minutes or seconds")
            })
        })
        .transpose()?;
    if let Some(dpv) = constraint.purpose_dpv.as_deref() {
        if !dpv.starts_with("https://w3id.org/dpv") || !plain_text(dpv, MAX_TOKEN_CHARS) {
            return Err(refused("purpose_dpv must be a DPV IRI"));
        }
    }
    if let Some(budgets) = &constraint.budgets {
        let values = [
            budgets.requests_per_hour,
            budgets.rows_per_day,
            budgets.raw_rows_per_day,
        ];
        if values.iter().all(Option::is_none) {
            return Err(refused("budgets must name at least one budget"));
        }
        if values.contains(&Some(0)) {
            return Err(refused("every budget must be at least 1"));
        }
    }
    if let Some(step_up) = &constraint.step_up {
        if step_up.required || step_up.acr.is_some() {
            return Err(refused("step-up is not offered yet"));
        }
    }
    match constraint.host_profile_required.as_deref() {
        None | Some("none") => {}
        Some(_) => return Err(refused("a Host Profile requirement is not offered yet")),
    }
    Ok(CheckedConstraints {
        max_tier: constraint.max_tier,
        representation: constraint.representation,
        lifetime_seconds,
        purpose_dpv: constraint.purpose_dpv,
        budgets: constraint.budgets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth2_server::authorization_details::tests::{access, constraints, resolved, MCP};
    use serde_json::json;

    fn refusal(details: &Value) -> String {
        let error = resolved(details).expect_err("refused");
        assert_eq!(error.error, "invalid_authorization_details", "{error:?}");
        error.error_description.unwrap_or_default()
    }

    fn assert_refused(details: &Value, why: &str) {
        let description = refusal(details);
        assert!(
            description.contains(why),
            "bug: expected a refusal naming {why:?}, got {description:?}"
        );
    }

    #[test]
    fn malformed_parameters_are_refused() {
        for raw in ["", "{}", "[]", "not json", "[1]", "[[]]"] {
            let error = parse_authorization_details(raw).expect_err(raw);
            assert_eq!(error.error, "invalid_authorization_details", "{raw}");
        }
        let oversized = format!(
            "[{{\"type\":\"{DATA_ACCESS_TYPE}\",\"purpose_description\":\"{}\"}}]",
            "a".repeat(MAX_AUTHORIZATION_DETAILS_BYTES)
        );
        assert!(
            parse_authorization_details(&oversized).is_err(),
            "bug: an oversized parameter was read"
        );
    }

    #[test]
    fn an_unknown_or_missing_type_is_refused() {
        assert_refused(
            &json!([{"type": "https://example.test/other"}]),
            "does not support",
        );
        assert_refused(
            &json!([{"selection_preset": "training_coach"}]),
            "has no type",
        );
    }

    #[test]
    fn exactly_one_data_access_detail_is_resolved() {
        assert_refused(
            &json!([constraints(&json!({}))]),
            "exactly one https://pdpp.dev/data-access",
        );
        assert_refused(
            &json!([access(&json!({})), access(&json!({}))]),
            "exactly one https://pdpp.dev/data-access",
        );
    }

    #[test]
    fn critical_is_required_on_health_constraints() {
        let mut missing = constraints(&json!({}));
        if let Value::Object(members) = &mut missing {
            members.remove("critical");
        }
        assert_refused(&json!([access(&json!({})), missing]), "critical: true");
        assert_refused(
            &json!([access(&json!({})), constraints(&json!({"critical": false}))]),
            "critical: true",
        );
    }

    #[test]
    fn applies_to_must_index_the_data_access_detail() {
        assert_refused(
            &json!([access(&json!({})), constraints(&json!({"applies_to": 1}))]),
            "applies_to",
        );
        assert_refused(
            &json!([access(&json!({})), constraints(&json!({"applies_to": 7}))]),
            "applies_to",
        );
        // The data-access detail need not come first.
        assert!(
            resolved(&json!([
                constraints(&json!({"applies_to": 1})),
                access(&json!({}))
            ]))
            .is_ok(),
            "bug: a data-access detail after its constraints was refused"
        );
        assert_refused(
            &json!([
                access(&json!({})),
                constraints(&json!({})),
                constraints(&json!({}))
            ]),
            "at most one health-constraints",
        );
    }

    #[test]
    fn only_the_two_presets_are_granted() {
        assert_refused(
            &json!([access(&json!({"selection_preset": "clinician_summary"}))]),
            "training_coach or sleep_coach",
        );
        let mut none = access(&json!({}));
        if let Value::Object(members) = &mut none {
            members.remove("selection_preset");
        }
        assert_refused(&json!([none]), "selection_preset");
    }

    #[test]
    fn explicit_streams_and_raw_streams_are_refused() {
        assert_refused(
            &json!([access(
                &json!({"streams": [{"name": "health.vault.sleep.summary"}]})
            )]),
            "explicit streams",
        );
        assert_refused(
            &json!([
                access(&json!({})),
                constraints(&json!({"raw_streams": ["health.vault.workout.raw"]}))
            ]),
            "raw_streams",
        );
    }

    #[test]
    fn a_source_must_be_the_tokens_resource() {
        assert_refused(
            &json!([access(
                &json!({"source": {"id": "https://other.example.test/mcp"}})
            )]),
            "source.id",
        );
        let mut none = access(&json!({}));
        if let Value::Object(members) = &mut none {
            members.remove("source");
        }
        assert_refused(&json!([none]), "source");
    }

    #[test]
    fn single_use_and_unknown_access_modes_are_refused() {
        assert_refused(
            &json!([access(&json!({"access_mode": "single_use"}))]),
            "single_use",
        );
        assert_refused(
            &json!([access(&json!({"access_mode": "forever"}))]),
            "access_mode",
        );
    }

    #[test]
    fn a_tier_above_t1_is_refused() {
        for tier in ["T2", "T3"] {
            assert_refused(
                &json!([access(&json!({})), constraints(&json!({"max_tier": tier}))]),
                "max_tier above T1",
            );
        }
        assert_refused(
            &json!([access(&json!({})), constraints(&json!({"max_tier": "T9"}))]),
            "authorization_details[1]",
        );
    }

    #[test]
    fn a_lifetime_must_be_a_fixed_length_duration() {
        for lifetime in ["P1M", "P1Y", "30 days", "P0D"] {
            assert_refused(
                &json!([
                    access(&json!({})),
                    constraints(&json!({"lifetime": lifetime}))
                ]),
                "lifetime",
            );
        }
    }

    #[test]
    fn total_rows_and_empty_or_zero_budgets_are_refused() {
        assert_refused(
            &json!([
                access(&json!({})),
                constraints(&json!({"budgets": {"total_rows": 200}}))
            ]),
            "total_rows",
        );
        assert_refused(
            &json!([access(&json!({})), constraints(&json!({"budgets": {}}))]),
            "at least one budget",
        );
        assert_refused(
            &json!([
                access(&json!({})),
                constraints(&json!({"budgets": {"rows_per_day": 0}}))
            ]),
            "at least 1",
        );
        assert_refused(
            &json!([
                access(&json!({})),
                constraints(&json!({"budgets": {"per_minute": 3}}))
            ]),
            "unknown field",
        );
    }

    #[test]
    fn code_filter_step_up_host_profile_and_egress_are_refused() {
        let cases = [
            (
                json!({"code_filter": {"system": "http://loinc.org", "codes": ["4548-4"]}}),
                "code_filter",
            ),
            (json!({"step_up": {"required": true}}), "step-up"),
            (
                json!({"step_up": {"required": false, "acr": "owner_verified"}}),
                "step-up",
            ),
            (json!({"host_profile_required": "t2"}), "Host Profile"),
            (json!({"egress": "typed_only"}), "egress"),
        ];
        for (extra, why) in cases {
            assert_refused(&json!([access(&json!({})), constraints(&extra)]), why);
        }
    }

    #[test]
    fn time_range_is_refused() {
        assert_refused(
            &json!([access(
                &json!({"time_range": {"since": "2026-01-01T00:00:00Z"}})
            )]),
            "time_range",
        );
    }

    #[test]
    fn as_set_members_are_refused_on_either_detail() {
        for member in AS_SET_MEMBERS {
            let extra = json!({ member: {"basis": "x"} });
            assert_refused(
                &json!([access(&json!({})), constraints(&extra)]),
                "set by the authorization server",
            );
            assert_refused(&json!([access(&extra)]), "set by the authorization server");
        }
    }

    #[test]
    fn unknown_members_are_refused() {
        assert_refused(
            &json!([access(&json!({"locations": [MCP]}))]),
            "unknown field",
        );
        assert_refused(
            &json!([access(&json!({})), constraints(&json!({"priority": 1}))]),
            "unknown field",
        );
        assert_refused(
            &json!([access(
                &json!({"client_claims": {"commitments": [], "extra": 1}})
            )]),
            "unknown field",
        );
    }

    #[test]
    fn the_client_words_are_bounded_plain_text() {
        assert_refused(
            &json!([access(
                &json!({"purpose_description": "a".repeat(MAX_PURPOSE_CHARS + 1)})
            )]),
            "purpose_description",
        );
        assert_refused(
            &json!([access(&json!({"purpose_description": "line\u{0007}bell"}))]),
            "purpose_description",
        );
        let commitments: Vec<String> = (0..=MAX_COMMITMENTS).map(|i| format!("c{i}")).collect();
        assert_refused(
            &json!([access(
                &json!({"client_claims": {"commitments": commitments}})
            )]),
            "commitments",
        );
        assert_refused(
            &json!([access(
                &json!({"retention": {"max_duration": "ninety days", "on_expiry": "delete"}})
            )]),
            "retention",
        );
        assert_refused(
            &json!([
                access(&json!({})),
                constraints(&json!({"purpose_dpv": "https://example.test/purpose"}))
            ]),
            "purpose_dpv",
        );
    }
}
