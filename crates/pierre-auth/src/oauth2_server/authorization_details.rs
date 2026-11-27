// ABOUTME: RFC 9396 authorization_details for owl's presets — parsed, narrowed and resolved to the one document a grant stores
// ABOUTME: Whatever the server cannot enforce is refused with invalid_authorization_details, never stored and ignored
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Authorization details (RFC 9396, owl PDPP §3–§4)
//!
//! A client may ask for less than everything: one `https://pdpp.dev/data-access`
//! detail naming a preset (`training_coach`, `sleep_coach`, barrage's frozen
//! stream sets), optionally narrowed by one
//! `https://dravr.ai/pdpp/health-profile/v0/health-constraints` detail.
//!
//! Resolution happens in two steps:
//! - [`parse_authorization_details`] and [`resolve`] run before the athlete
//!   signs in, so a refusal reaches the client as RFC 6749 §4.1.2.1 sends it.
//!   They expand the preset and take the narrower of its value and the
//!   client's for every field.
//! - [`ResolvedDetails::freeze`] runs when the athlete approves: the data
//!   window's `since` becomes the athlete's local midnight `window` days back,
//!   in UTC, and the lifetime becomes the grant's `expires_at`.
//!
//! [`ResolvedDetails`] is the document the grant stores, the token response
//! echoes (RFC 9396 §7) and introspection returns: a JSON array of the two
//! resolved details, serialized from typed structs in a fixed field order. It
//! names the preset and never a stream (stream names are not published, E5).
//!
//! Every member the server cannot enforce yet is refused, which is what
//! `critical: true` asks: `raw_streams`, `code_filter`, `egress`; so are the
//! members only the server sets and every unknown member. A step-up preset
//! (`one_off_question`, owl §3.1) alone admits explicit `streams` among its
//! own, their `time_range`, `single_use`, `total_rows`, `max_tier: T2`, a
//! step-up and a Host Profile requirement: everything else asks for them in
//! vain, and `max_tier` above T1 is refused for it.

pub use dravr_barrage::{Preset, Stream, Tier};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::models::OAuth2Error;

/// The agent-write detail: records a client may propose (owl PDPP §4.5)
mod agent_write;
/// The one typed reader of a grant's stored document
mod document;
/// ISO 8601 durations of fixed-length units
mod duration;
/// A client's request, parsed and checked against the field table
mod parse;
/// The resolved document a grant stores
mod resolved;

pub use agent_write::{
    agent_write_review_digest, is_agent_write_scope, AgentWriteRate, AgentWriteTarget,
    ResolvedAgentWrite, SenderConstraint, StepUpResult, AGENT_WRITE_RESOURCE_PATH,
    AGENT_WRITE_SCOPE, AGENT_WRITE_TYPE, WRITE_STEP_UP_RECORDS_PER_DAY,
};
pub use document::GrantDocument;
pub use parse::{parse_authorization_details, ParsedDetails, RequestedDetails};
pub use resolved::{athlete_zone, preset_defaults, resolve, review_digest, ResolvedDetails};

/// The PDPP Core detail type a client asks for data with.
pub const DATA_ACCESS_TYPE: &str = "https://pdpp.dev/data-access";

/// Dravr's health-profile detail type, narrowing one data-access detail.
pub const HEALTH_CONSTRAINTS_TYPE: &str =
    "https://dravr.ai/pdpp/health-profile/v0/health-constraints";

/// The detail types this server resolves, as discovery lists them.
pub const AUTHORIZATION_DETAILS_TYPES: [&str; 3] =
    [DATA_ACCESS_TYPE, HEALTH_CONSTRAINTS_TYPE, AGENT_WRITE_TYPE];

/// The prefix owl's stream names carry on the wire (`health.vault.labs.summary`);
/// a name without it is read as barrage's own.
pub(crate) const STREAM_NAME_PREFIX: &str = "health.vault.";

/// The longest `authorization_details` parameter read, in bytes.
pub(crate) const MAX_AUTHORIZATION_DETAILS_BYTES: usize = 8 * 1024;

/// The refusal of a request's `authorization_details` (RFC 9396 §5).
fn refused(description: &str) -> OAuth2Error {
    warn!("authorization_details refused: {description}");
    OAuth2Error::invalid_authorization_details(description)
}

/// How much of a preset's data a grant reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    /// Summaries only: no raw sample stream
    Summary,
    /// Summaries and the preset's own raw streams
    SummaryAndRaw,
}

/// How long a grant may be used (owl §3.1 `access_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Read repeatedly while the grant stands
    Continuous,
    /// Consumed by the first call that reads data under it
    SingleUse,
}

/// The resource server a detail's data is read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// The resource identifier: the resource the tokens are for
    pub id: String,
}

/// What a client commits to, in its own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientClaims {
    /// Each commitment, as the client wrote it
    pub commitments: Vec<String>,
}

/// How long a client says it keeps what it reads, in its own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    /// An ISO 8601 duration
    pub max_duration: String,
    /// What it does with the data then (`delete`, …)
    pub on_expiry: String,
}

/// A grant's budgets: the narrower of the preset's and the client's.
///
/// A budget that is `None` has no limit of its own: a one-off question is
/// budgeted in rows over its life, not per day. The members a grant has no
/// value for are left out of the stored document, so a document written
/// before a member existed reads back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    /// Calls per hour
    pub requests_per_hour: u32,
    /// Rows served per UTC day
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_per_day: Option<u32>,
    /// Raw time-series samples served per UTC day
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_rows_per_day: Option<u32>,
    /// Rows served over the grant's whole life (owl §3.1 `total_rows`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_rows: Option<u32>,
}

#[cfg(test)]
mod tests {
    //! Fixtures the parse and resolved tests share: the two details a client
    //! sends, and their resolution for tokens bound to [`MCP`].

    use super::{parse_authorization_details, resolve, ResolvedDetails};
    use super::{DATA_ACCESS_TYPE, HEALTH_CONSTRAINTS_TYPE};
    use crate::oauth2_server::models::OAuth2Error;
    use serde_json::{json, Value};

    /// The resource the fixtures' tokens are for.
    pub(super) const MCP: &str = "https://mcp.example.test/mcp";

    /// A training-coach data-access detail, `extra`'s members laid over it.
    pub(super) fn access(extra: &Value) -> Value {
        let mut detail = json!({
            "type": DATA_ACCESS_TYPE,
            "source": {"id": MCP},
            "selection_preset": "training_coach",
        });
        if let (Value::Object(detail), Value::Object(extra)) = (&mut detail, extra) {
            detail.extend(extra.clone());
        }
        detail
    }

    /// A critical health-constraints detail applying to detail 0, `extra`'s
    /// members laid over it.
    pub(super) fn constraints(extra: &Value) -> Value {
        let mut detail = json!({
            "type": HEALTH_CONSTRAINTS_TYPE,
            "applies_to": 0,
            "critical": true,
        });
        if let (Value::Object(detail), Value::Object(extra)) = (&mut detail, extra) {
            detail.extend(extra.clone());
        }
        detail
    }

    /// `details` parsed and resolved for tokens bound to [`MCP`].
    pub(super) fn resolved(details: &Value) -> Result<ResolvedDetails, OAuth2Error> {
        let parsed = parse_authorization_details(&details.to_string())?;
        let access = parsed
            .data_access
            .ok_or_else(|| OAuth2Error::invalid_authorization_details("no data-access detail"))?;
        resolve(&access, &[MCP])
    }
}
