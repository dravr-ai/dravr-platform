// ABOUTME: The step-up scope (pdpp:preset:one_off_question pdpp:stream:<name>) a 403 names and an authorization request maps
// ABOUTME: The resource server builds the challenge from it; the authorization server reads it back as a one_off_question request

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The step-up scope (owl §7.6 row 2, carnet#1544 slice 4b).
//!
//! A `tools/call` a step-up would cure is answered `403` with
//! `WWW-Authenticate: Bearer error="insufficient_scope", scope="…"`, the
//! scope naming the token's own grant, [`STEP_UP_PRESET_SCOPE`] and one
//! [`STEP_UP_STREAM_SCOPE`] per stream the call reads
//! ([`challenge_scope`]). A host following MCP's step-up flow starts an
//! authorization request with that scope; the authorization server reads the
//! `pdpp:` names out of it ([`split`]) and asks for a `one_off_question`
//! detail over those streams instead ([`one_off_details`]), with the step-up
//! and the Host Profile T2 every T2 grant needs. The rest of the scope is the
//! grant the code carries, which a step-up never widens past its parent's.
//!
//! A write step-up (owl §7.4, last bullet) names `health:write:<stream>`
//! instead ([`WRITE_STREAM_SCOPE`], [`write_challenge_scope`]): the server
//! asks for an agent-write detail over those streams
//! ([`ResolvedAgentWrite::for_write_scope`]), a child of the MCP package. A
//! request names reading or writing, never both, and never
//! [`AGENT_WRITE_SCOPE`], which only the server sets.
//!
//! Both halves are one vocabulary, here, so the challenge a host is sent is
//! the request the server reads.

use pierre_core::models::OWNER_VERIFIED_ACR;
use serde_json::{json, Value};

use dravr_barrage::agent_write::row_schema;

use super::authorization_details::{
    Preset, ResolvedAgentWrite, Stream, AGENT_WRITE_SCOPE, DATA_ACCESS_TYPE,
    HEALTH_CONSTRAINTS_TYPE, STREAM_NAME_PREFIX,
};
use super::models::OAuth2Error;

/// The scope naming a one-off question's preset.
pub const STEP_UP_PRESET_SCOPE: &str = "pdpp:preset:one_off_question";

/// The prefix of the scope naming one stream a one-off question reads, as
/// owl names it (`pdpp:stream:labs.summary`).
pub const STEP_UP_STREAM_SCOPE: &str = "pdpp:stream:";

/// The prefix of the scope naming one stream a write step-up proposes into
/// (`health:write:labs.summary`, owl §7.4).
pub const WRITE_STREAM_SCOPE: &str = "health:write:";

/// Every `pdpp:` name starts so.
const PDPP_SCOPE_PREFIX: &str = "pdpp:";

/// The grant a step-up request carries when its scope names nothing beside
/// the `pdpp:` names: reading, the least any parent grant holds.
const STEP_UP_DEFAULT_SCOPE: &str = "fitness:read";

/// An authorization request's scope with its step-up names read out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepUpScope {
    /// The rest of the scope: the grant the code carries
    pub scope: String,
    /// The streams the one-off question reads, as owl names them; empty
    /// for a write step-up
    pub streams: Vec<Stream>,
    /// The streams a write step-up proposes into; empty for a read one
    pub writes: Vec<Stream>,
}

/// The scope a `403` names for a call `granted` covers but whose `streams` a
/// step-up would open: the grant itself, so a host re-authorizing keeps it,
/// then the preset and each stream.
#[must_use]
pub fn challenge_scope(granted: &str, streams: &[Stream]) -> String {
    let mut names: Vec<String> = granted.split_whitespace().map(str::to_owned).collect();
    names.push(STEP_UP_PRESET_SCOPE.to_owned());
    names.extend(
        streams
            .iter()
            .map(|stream| format!("{STEP_UP_STREAM_SCOPE}{}", stream.name())),
    );
    names.join(" ")
}

/// The scope a `403` names for a write tool call whose `streams` no live
/// agent-write child covers: the grant itself, then one
/// [`WRITE_STREAM_SCOPE`] name per stream.
#[must_use]
pub fn write_challenge_scope(granted: &str, streams: &[Stream]) -> String {
    let mut names: Vec<String> = granted.split_whitespace().map(str::to_owned).collect();
    names.extend(
        streams
            .iter()
            .map(|stream| format!("{WRITE_STREAM_SCOPE}{}", stream.name())),
    );
    names.join(" ")
}

/// The scope a step-up child of a grant held for `parent_scope` is stored
/// under: the parent's, then [`STEP_UP_PRESET_SCOPE`].
///
/// Never the parent's own: the previous release's consent upsert names
/// `idx_oauth_client_grants_active`, one live row per (user, tenant, client,
/// scope) over children and parents alike, and a child sharing its parent's
/// scope would collide with it there (migration `20261126200000`, expand
/// then contract). A parent never holds a `pdpp:` name ([`split`] reads them
/// out), so no parent's scope is ever a child's.
#[must_use]
pub fn child_grant_scope(parent_scope: &str) -> String {
    format!("{parent_scope} {STEP_UP_PRESET_SCOPE}")
}

/// The step-up an authorization request's `scope` names: `None` when it
/// names no `pdpp:` scope, the request then read as any other.
///
/// # Errors
/// `invalid_scope` for a `pdpp:` name this server does not read, a stream it
/// does not know, a stream without the preset or the preset without a stream.
pub fn split(scope: Option<&str>) -> Result<Option<StepUpScope>, OAuth2Error> {
    let Some(scope) = scope else {
        return Ok(None);
    };
    let (asked, rest): (Vec<&str>, Vec<&str>) = scope.split_whitespace().partition(|name| {
        name.starts_with(PDPP_SCOPE_PREFIX) || name.starts_with(WRITE_STREAM_SCOPE)
    });
    if asked.is_empty() {
        return Ok(None);
    }
    let mut preset = false;
    let mut streams = Vec::new();
    let mut writes = Vec::new();
    for name in asked {
        if name == STEP_UP_PRESET_SCOPE {
            preset = true;
            continue;
        }
        if name == AGENT_WRITE_SCOPE {
            return Err(OAuth2Error::invalid_scope(&format!(
                "{AGENT_WRITE_SCOPE} is set by the authorization server; ask \
                 {WRITE_STREAM_SCOPE}<name> or an agent-write detail"
            )));
        }
        if let Some(stream) = name.strip_prefix(WRITE_STREAM_SCOPE) {
            let stream = Stream::from_name(stream)
                .filter(|stream| row_schema(*stream).is_some())
                .ok_or_else(|| {
                    OAuth2Error::invalid_scope(&format!(
                        "'{name}': no record of that stream can be proposed"
                    ))
                })?;
            if !writes.contains(&stream) {
                writes.push(stream);
            }
            continue;
        }
        let stream = name
            .strip_prefix(STEP_UP_STREAM_SCOPE)
            .and_then(Stream::from_name)
            .ok_or_else(|| OAuth2Error::invalid_scope(&format!("Unknown scope '{name}'")))?;
        if !streams.contains(&stream) {
            streams.push(stream);
        }
    }
    if !writes.is_empty() && (preset || !streams.is_empty()) {
        return Err(OAuth2Error::invalid_scope(
            "A step-up asks to read or to propose records, never both at once",
        ));
    }
    if writes.is_empty() && (!preset || streams.is_empty()) {
        return Err(OAuth2Error::invalid_scope(&format!(
            "A step-up scope names {STEP_UP_PRESET_SCOPE} and at least one \
             {STEP_UP_STREAM_SCOPE}<name>"
        )));
    }
    let scope = if rest.is_empty() {
        STEP_UP_DEFAULT_SCOPE.to_owned()
    } else {
        rest.join(" ")
    };
    Ok(Some(StepUpScope {
        scope,
        streams,
        writes,
    }))
}

/// The `authorization_details` a write step-up asks for: an agent-write
/// detail over its streams, bound by the MCP package; `None` for a read
/// step-up.
///
/// # Errors
/// `invalid_scope` for a stream no record of which can be proposed, or a
/// detail that does not serialize.
pub fn agent_write_details(step_up: &StepUpScope) -> Result<Option<String>, OAuth2Error> {
    if step_up.writes.is_empty() {
        return Ok(None);
    }
    ResolvedAgentWrite::for_write_scope(&step_up.writes)?
        .to_document()
        .map(Some)
        .map_err(|_| OAuth2Error::server_error("Failed to build the write step-up's details"))
}

/// The `authorization_details` a step-up scope asks for, read from `source`.
///
/// A single-use `one_off_question` over its streams, at most T2, after a
/// step-up `owner_verified`, for a host declaring a Host Profile T2.
#[must_use]
pub fn one_off_details(step_up: &StepUpScope, source: &str) -> String {
    let streams: Vec<Value> = step_up
        .streams
        .iter()
        .map(|stream| {
            json!({
                "name": format!("{STREAM_NAME_PREFIX}{}", stream.name()),
                "necessity": "required",
            })
        })
        .collect();
    json!([
        {
            "type": DATA_ACCESS_TYPE,
            "source": {"id": source},
            "selection_preset": Preset::OneOffQuestion.name(),
            "access_mode": "single_use",
            "streams": streams,
        },
        {
            "type": HEALTH_CONSTRAINTS_TYPE,
            "applies_to": 0,
            "critical": true,
            "max_tier": "T2",
            "step_up": {"required": true, "acr": OWNER_VERIFIED_ACR},
            "host_profile_required": "t2",
        },
    ])
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth2_server::authorization_details::parse_authorization_details;

    #[test]
    fn a_scope_naming_no_step_up_is_read_as_before() {
        assert_eq!(split(None).ok(), Some(None), "bug: no scope");
        assert_eq!(
            split(Some("fitness:read profile:read")).ok(),
            Some(None),
            "bug: a plain scope"
        );
    }

    #[test]
    fn the_challenge_scope_reads_back_as_its_step_up() {
        let challenge = challenge_scope("fitness:read profile:read", &[Stream::Labs]);
        assert_eq!(
            challenge,
            "fitness:read profile:read pdpp:preset:one_off_question pdpp:stream:labs.summary",
            "bug: the challenge"
        );
        assert_eq!(
            split(Some(&challenge)).ok().flatten(),
            Some(StepUpScope {
                scope: "fitness:read profile:read".to_owned(),
                streams: vec![Stream::Labs],
                writes: Vec::new(),
            }),
            "bug: the challenge does not read back"
        );
    }

    #[test]
    fn a_step_up_scope_alone_carries_reading() {
        let step_up = split(Some(
            "pdpp:preset:one_off_question pdpp:stream:labs.summary",
        ))
        .ok()
        .flatten()
        .unwrap();
        assert_eq!(step_up.scope, STEP_UP_DEFAULT_SCOPE, "bug: the default");
    }

    #[test]
    fn an_incomplete_or_unknown_step_up_scope_is_refused() {
        for scope in [
            "pdpp:preset:one_off_question",
            "pdpp:stream:labs.summary",
            "pdpp:preset:one_off_question pdpp:stream:nothing.summary",
            "pdpp:preset:training_coach pdpp:stream:labs.summary",
        ] {
            assert!(split(Some(scope)).is_err(), "bug: {scope} was read");
        }
    }

    #[test]
    fn the_details_parse_as_a_one_off_question_over_the_streams() {
        let step_up = split(Some(
            "pdpp:preset:one_off_question pdpp:stream:labs.summary",
        ))
        .ok()
        .flatten()
        .unwrap();
        let details = one_off_details(&step_up, "https://mcp.example.test/mcp");
        let requested = parse_authorization_details(&details)
            .unwrap()
            .data_access
            .unwrap();
        assert!(
            requested.requires_step_up(),
            "bug: no one-off question asked"
        );
    }

    #[test]
    fn a_write_challenge_reads_back_and_builds_an_agent_write_detail() {
        let challenge = write_challenge_scope("fitness:read", &[Stream::Labs, Stream::Sleep]);
        assert_eq!(
            challenge,
            "fitness:read health:write:labs.summary health:write:sleep.summary"
        );
        let step_up = split(Some(&challenge)).unwrap().unwrap();
        assert_eq!(
            step_up.scope, "fitness:read",
            "bug: health:write leaked into the scope"
        );
        assert_eq!(step_up.writes, vec![Stream::Labs, Stream::Sleep]);
        assert!(step_up.streams.is_empty());
        let details = agent_write_details(&step_up).unwrap().unwrap();
        let parsed = parse_authorization_details(&details).unwrap();
        assert!(parsed.data_access.is_none());
        let write = parsed.agent_write.unwrap();
        assert!(write.target(Stream::Labs).is_some() && write.target(Stream::Sleep).is_some());
        let read = split(Some(
            "pdpp:preset:one_off_question pdpp:stream:labs.summary",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(agent_write_details(&read).unwrap(), None);
    }

    #[test]
    fn the_server_set_scope_and_mixed_or_unwritable_write_scopes_are_refused() {
        for scope in [
            "pdpp:agent_write",
            "fitness:read pdpp:agent_write",
            "health:write:labs.summary pdpp:preset:one_off_question pdpp:stream:labs.summary",
            "health:write:workout.raw",
            "health:write:nothing.summary",
            "health:write:agent.pending",
        ] {
            let error = split(Some(scope)).expect_err(scope);
            assert_eq!(error.error, "invalid_scope", "bug: {scope}");
        }
    }
}
