// ABOUTME: A consent grant's receipts: the issue, revoke and expire events of an oauth_client_grants row, and their payload
// ABOUTME: The payload is serialized once, in a fixed field order, when the event is written; its SHA-256 is what the record_log names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Grant receipts (carnet#1544, design row 34, owl PDPP §8.1)
//!
//! Every event in a consent grant's life (an [`OAuthClientGrant`] row) is
//! recorded as one `oauth_grant_events` row, written by the statement that
//! starts or ends the grant, in its transaction. The row carries the
//! receipt's payload as the bytes that are hashed and signed: built here,
//! once, when the event is written, because a grant row hard-deleted with
//! its client is gone by the time the receipt is read.
//!
//! The payload is serialized from typed structs in a fixed field order (the
//! declaration order), as the deletion receipt's body is. It is not JCS: RFC
//! 8785's number and UTF-16 ordering rules are not applied, and a verifier
//! checks the stored bytes, never a re-serialization.
//!
//! A grant ends once: [`GrantEvent::ending`] decides whether an ending is a
//! `revoke` or, for a grant already past its expiry, an `expire`.

use dravr_tronc::dates::{SecondsFormat, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::OAuthClientGrant;
use crate::errors::{AppError, AppResult};

/// The payload format every receipt written here carries.
const GRANT_RECEIPT_VERSION: u32 = 0;

/// What happened to a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantEvent {
    /// The grant was given, or re-consented to in place.
    Issue,
    /// The grant was ended before its expiry.
    Revoke,
    /// The grant reached its expiry (whoever ended it after that).
    Expire,
}

impl GrantEvent {
    /// The stored value (`oauth_grant_events.event`), a persisted value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Revoke => "revoke",
            Self::Expire => "expire",
        }
    }

    /// The event a stored value names.
    ///
    /// # Errors
    /// A database error for a value no event stores.
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "issue" => Ok(Self::Issue),
            "revoke" => Ok(Self::Revoke),
            "expire" => Ok(Self::Expire),
            other => Err(AppError::database(format!(
                "oauth_grant_events.event holds an unknown event `{other}`"
            ))),
        }
    }

    /// Whether the event ends the grant (one per grant).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Revoke | Self::Expire)
    }

    /// The event, actor and time that record `grant` ended by `actor` at
    /// `ended_at`: a revoke, unless the grant was already past its expiry,
    /// which then ends as expired at its `expires_at`, by the AS (`auto`).
    /// A grant past its expiry ends as expired, whoever ends it first.
    #[must_use]
    pub fn ending(
        grant: &OAuthClientGrant,
        actor: GrantActor,
        ended_at: Timestamp,
    ) -> (Self, GrantActor, Timestamp) {
        match grant.expires_at {
            Some(expires_at) if expires_at <= ended_at => {
                (Self::Expire, GrantActor::Auto, expires_at)
            }
            _ => (Self::Revoke, actor, ended_at),
        }
    }
}

/// Who caused a grant event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantActor {
    /// The athlete: the consent screen, Connected apps.
    User,
    /// The client: RFC 7009 revocation, the owner deleting its A2A client.
    Client,
    /// The AS itself: a standing policy, a sweep, a federated retirement,
    /// an expiry.
    Auto,
}

impl GrantActor {
    /// The stored value (`oauth_grant_events.actor`), a persisted value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Client => "client",
            Self::Auto => "auto",
        }
    }

    /// The actor a stored value names.
    ///
    /// # Errors
    /// A database error for a value no actor stores.
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "user" => Ok(Self::User),
            "client" => Ok(Self::Client),
            "auto" => Ok(Self::Auto),
            other => Err(AppError::database(format!(
                "oauth_grant_events.actor holds an unknown actor `{other}`"
            ))),
        }
    }
}

/// The standing policy that renewed a grant, as its receipt cites it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingPolicyRef {
    /// The policy's id (`oauth_standing_policies.id`)
    pub id: String,
    /// The review digest the policy was approved under
    pub consent_version: String,
}

/// Who approved a grant being stored: what its `issue` receipt states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantApproval {
    /// The athlete on the consent screen, or the AS on a standing policy
    pub actor: GrantActor,
    /// The policy that renewed the grant; `None` for the consent screen
    pub standing_policy: Option<StandingPolicyRef>,
}

impl GrantApproval {
    /// The athlete approved the grant on the consent screen.
    #[must_use]
    pub const fn by_user() -> Self {
        Self {
            actor: GrantActor::User,
            standing_policy: None,
        }
    }
}

/// One stored grant event, a row of `oauth_grant_events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthGrantEvent {
    /// The receipt's id (`receipt_id` in its payload)
    pub id: String,
    /// The grant's tenant
    pub tenant_id: String,
    /// The athlete who holds the grant
    pub user_id: String,
    /// The grant (`oauth_client_grants.id`), which may since be deleted
    pub client_grant_id: String,
    /// What happened
    pub event: GrantEvent,
    /// Who caused it
    pub actor: GrantActor,
    /// When it happened
    pub at: Timestamp,
    /// The receipt payload, exactly the bytes hashed and signed
    pub payload: String,
    /// Lowercase hex SHA-256 of `payload`: the `record_log` entry's content
    pub payload_sha256: String,
    /// The compact JWS over `payload`; `None` until a signer signed it
    pub jws: Option<String>,
    /// When its `record_log` entry was appended; `None` until then
    pub logged_at: Option<Timestamp>,
}

/// How a receipt's `approval` member reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptApproval<'approval> {
    /// Who caused the event
    pub actor: GrantActor,
    /// The standing policy that renewed the grant, on its `issue`
    pub standing_policy: Option<&'approval StandingPolicyRef>,
    /// Whether the receipt is a baseline: an `issue` written for a grant
    /// given before receipts were
    pub baseline: bool,
}

impl ReceiptApproval<'_> {
    /// An event caused by `actor`, citing no policy.
    #[must_use]
    pub const fn by(actor: GrantActor) -> Self {
        Self {
            actor,
            standing_policy: None,
            baseline: false,
        }
    }
}

impl<'approval> ReceiptApproval<'approval> {
    /// The `issue` of a grant stored as `approval` says: by its approver,
    /// citing the standing policy that renewed it, if any.
    #[must_use]
    pub const fn from_approval(approval: &'approval GrantApproval) -> Self {
        Self {
            actor: approval.actor,
            standing_policy: approval.standing_policy.as_ref(),
            baseline: false,
        }
    }
}

impl<'approval> From<&'approval GrantApproval> for ReceiptApproval<'approval> {
    fn from(approval: &'approval GrantApproval) -> Self {
        Self::from_approval(approval)
    }
}

/// One receipt to build: the event, and what it states.
#[derive(Debug, Clone, Copy)]
pub struct GrantReceipt<'receipt> {
    /// The receipt's id, the event row's
    pub receipt_id: &'receipt str,
    /// What happened
    pub event: GrantEvent,
    /// When it happened
    pub at: Timestamp,
    /// The grant as stored when it happened
    pub grant: &'receipt OAuthClientGrant,
    /// Who caused it
    pub approval: ReceiptApproval<'receipt>,
    /// The grant's latest `issue` receipt, for a terminal event; `None`
    /// when the grant has none
    pub issue_receipt_id: Option<&'receipt str>,
}

impl GrantReceipt<'_> {
    /// The payload, serialized once in its fixed field order, and the
    /// lowercase hex SHA-256 of those bytes.
    ///
    /// # Errors
    /// An internal error if the payload does not serialize, which a tree of
    /// strings, booleans and JSON values never fails to.
    pub fn payload(&self) -> AppResult<(String, String)> {
        let grant = self.grant;
        let details = grant.authorization_details.as_deref().map(|document| {
            // The stored text is what the athlete approved; one that is not
            // JSON is stated as the text it is, never dropped.
            serde_json::from_str::<Value>(document)
                .unwrap_or_else(|_| Value::String(document.to_owned()))
        });
        let claims = details
            .as_ref()
            .and_then(|details| details_member(details, "client_claims"));
        let client_claims_digest = claims.map(sha256_of_value).transpose()?;
        let rendered_claims = claims.map(|claims| {
            claims
                .get("commitments")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()))
        });
        let dpv_purpose = details
            .as_ref()
            .and_then(|details| details_member(details, "purpose_dpv"))
            .and_then(Value::as_str);
        let payload = Payload {
            receipt_version: GRANT_RECEIPT_VERSION,
            receipt_id: self.receipt_id,
            event: self.event,
            at: receipt_time(self.at),
            grant: PayloadGrant {
                id: &grant.id,
                subject: format!("urn:dravr:user:{}", grant.user_id),
                client_id: &grant.client_id,
                scope: &grant.scope,
                authorization_details: details.as_ref(),
                granted_at: receipt_time(grant.granted_at),
                expires_at: grant.expires_at.map(receipt_time),
            },
            consent_evidence: ConsentEvidence {
                review_digest: grant.consent_version.as_deref(),
                locale: grant.consent_locale.as_deref(),
                client_claims_digest,
                rendered_claims,
            },
            approval: PayloadApproval {
                actor: self.approval.actor,
                standing_policy_ref: self.approval.standing_policy,
                baseline: self.approval.baseline,
            },
            issue_receipt_id: if self.event.is_terminal() {
                IssueReceiptRef::Ends(self.issue_receipt_id)
            } else {
                IssueReceiptRef::Absent
            },
            host_profile: HostProfile {
                basis: "not_required",
            },
            dpv_purpose,
        };
        let text = serde_json::to_string(&payload)
            .map_err(|e| AppError::internal(format!("Failed to encode a grant receipt: {e}")))?;
        let digest = Sha256::digest(text.as_bytes());
        let digest = format!("{digest:x}");
        Ok((text, digest))
    }
}

/// A receipt's time: RFC 3339, microseconds, `Z`.
fn receipt_time(at: Timestamp) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// `key` in a stored authorization details document, as a receipt's
/// payload embeds it: the RFC 9396 array's members are searched in order,
/// the first that holds it answers.
#[must_use]
pub fn details_member<'doc>(details: &'doc Value, key: &str) -> Option<&'doc Value> {
    match details {
        Value::Array(members) => members.iter().find_map(|member| member.get(key)),
        Value::Object(_) => details.get(key),
        _ => None,
    }
}

/// Lowercase hex SHA-256 of `value` serialized compactly, in its stored
/// member order.
fn sha256_of_value(value: &Value) -> AppResult<String> {
    let text = serde_json::to_string(value)
        .map_err(|e| AppError::internal(format!("Failed to encode client claims: {e}")))?;
    let digest = Sha256::digest(text.as_bytes());
    Ok(format!("{digest:x}"))
}

/// The receipt payload (owl §8.1), in its signed field order.
#[derive(Serialize)]
struct Payload<'receipt> {
    receipt_version: u32,
    receipt_id: &'receipt str,
    event: GrantEvent,
    at: String,
    grant: PayloadGrant<'receipt>,
    consent_evidence: ConsentEvidence<'receipt>,
    approval: PayloadApproval<'receipt>,
    /// Present (possibly null) on a terminal event, absent on an issue
    #[serde(skip_serializing_if = "IssueReceiptRef::is_absent")]
    issue_receipt_id: IssueReceiptRef<'receipt>,
    host_profile: HostProfile,
    #[serde(skip_serializing_if = "Option::is_none")]
    dpv_purpose: Option<&'receipt str>,
}

/// What a receipt says of the grant's issue receipt: nothing on an issue,
/// and on a revoke or expire the issue receipt's id, or null for a grant no
/// issue receipt was written for.
enum IssueReceiptRef<'receipt> {
    /// An issue receipt names none: the member is left out
    Absent,
    /// A terminal receipt names the grant's issue receipt, if it has one
    Ends(Option<&'receipt str>),
}

impl IssueReceiptRef<'_> {
    /// Whether the member is left out of the payload.
    const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

impl Serialize for IssueReceiptRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Absent => serializer.serialize_none(),
            Self::Ends(id) => id.serialize(serializer),
        }
    }
}

/// The grant, as stored when the event happened.
#[derive(Serialize)]
struct PayloadGrant<'receipt> {
    id: &'receipt str,
    subject: String,
    client_id: &'receipt str,
    scope: &'receipt str,
    authorization_details: Option<&'receipt Value>,
    granted_at: String,
    expires_at: Option<String>,
}

/// What the athlete was shown and approved.
#[derive(Serialize)]
struct ConsentEvidence<'receipt> {
    review_digest: Option<&'receipt str>,
    locale: Option<&'receipt str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_claims_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rendered_claims: Option<Value>,
}

/// Who caused the event.
#[derive(Serialize)]
struct PayloadApproval<'receipt> {
    actor: GrantActor,
    standing_policy_ref: Option<&'receipt StandingPolicyRef>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    baseline: bool,
}

/// The host attestation basis: every grant here reads at T0–T1, which
/// requires none.
#[derive(Serialize)]
struct HostProfile {
    basis: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instant `seconds` after the epoch.
    fn at(seconds: i64) -> AppResult<Timestamp> {
        Timestamp::from_epoch_seconds(seconds)
            .map_err(|e| AppError::internal(format!("test instant {seconds}: {e}")))
    }

    /// `text` read as JSON.
    fn json(text: &str) -> AppResult<Value> {
        serde_json::from_str(text).map_err(|e| AppError::internal(format!("payload JSON: {e}")))
    }

    fn grant(details: Option<&str>, expires_at: Option<Timestamp>) -> AppResult<OAuthClientGrant> {
        Ok(OAuthClientGrant {
            id: "grant-1".to_owned(),
            user_id: "user-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            client_id: "https://client.example/meta.json".to_owned(),
            scope: "read".to_owned(),
            granted_at: at(1_700_000_000)?,
            revoked_at: None,
            consent_version: Some("digest".to_owned()),
            consent_locale: Some("en".to_owned()),
            authorization_details: details.map(str::to_owned),
            expires_at,
        })
    }

    /// Where each of `keys` first appears in `text` as a JSON member; an
    /// error naming the first one missing.
    fn positions(text: &str, keys: &[&str]) -> AppResult<Vec<usize>> {
        keys.iter()
            .map(|key| {
                text.find(&format!("\"{key}\":")).ok_or_else(|| {
                    AppError::internal(format!("bug: `{key}` is missing from {text}"))
                })
            })
            .collect()
    }

    #[test]
    fn the_payload_keeps_its_field_order() -> AppResult<()> {
        let details = r#"[{"type":"health_data_access","client_claims":{"commitments":["We never sell it"]}},{"type":"health_constraints","purpose_dpv":"https://w3id.org/dpv#ServiceProvision"}]"#;
        let grant = grant(Some(details), None)?;
        let (text, digest) = GrantReceipt {
            receipt_id: "receipt-2",
            event: GrantEvent::Revoke,
            at: grant.granted_at,
            grant: &grant,
            approval: ReceiptApproval::by(GrantActor::User),
            issue_receipt_id: Some("receipt-1"),
        }
        .payload()?;
        let top = positions(
            &text,
            &[
                "receipt_version",
                "receipt_id",
                "event",
                "at",
                "grant",
                "consent_evidence",
                "approval",
                "issue_receipt_id",
                "host_profile",
                "dpv_purpose",
            ],
        )?;
        assert!(
            top.windows(2).all(|pair| pair[0] < pair[1]),
            "bug: the payload's members are out of their signed order: {text}"
        );
        let inner = positions(
            &text,
            &[
                "subject",
                "client_id",
                "scope",
                "authorization_details",
                "granted_at",
                "expires_at",
            ],
        )?;
        assert!(
            inner.windows(2).all(|pair| pair[0] < pair[1]),
            "bug: the grant's members are out of their signed order: {text}"
        );
        assert_eq!(
            digest,
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "bug: the digest is not of the stored bytes"
        );
        let value: Value = json(&text)?;
        assert_eq!(value["receipt_version"], 0, "bug: the version moved");
        assert_eq!(
            value["grant"]["subject"], "urn:dravr:user:user-1",
            "bug: subject"
        );
        assert_eq!(
            value["consent_evidence"]["rendered_claims"][0], "We never sell it",
            "bug: the claims are not stated as shown"
        );
        assert_eq!(
            value["consent_evidence"]["client_claims_digest"],
            format!(
                "{:x}",
                Sha256::digest(br#"{"commitments":["We never sell it"]}"#)
            ),
            "bug: the claims digest is not of their compact form"
        );
        assert_eq!(
            value["dpv_purpose"], "https://w3id.org/dpv#ServiceProvision",
            "bug: the DPV purpose is not the details'"
        );
        assert_eq!(value["host_profile"]["basis"], "not_required", "bug: basis");
        assert!(
            value["approval"].get("baseline").is_none(),
            "bug: a non-baseline receipt states a baseline flag"
        );
        Ok(())
    }

    #[test]
    fn an_issue_names_no_issue_receipt_and_a_plain_grant_no_details() -> AppResult<()> {
        let grant = grant(None, None)?;
        let (text, _) = GrantReceipt {
            receipt_id: "receipt-1",
            event: GrantEvent::Issue,
            at: grant.granted_at,
            grant: &grant,
            approval: ReceiptApproval {
                actor: GrantActor::Auto,
                standing_policy: None,
                baseline: true,
            },
            issue_receipt_id: None,
        }
        .payload()?;
        let value: Value = json(&text)?;
        assert!(
            value.get("issue_receipt_id").is_none(),
            "bug: an issue names an issue receipt"
        );
        assert!(
            value.get("dpv_purpose").is_none(),
            "bug: an absent purpose is stated"
        );
        assert!(
            value["grant"]["authorization_details"].is_null(),
            "bug: a plain grant states details"
        );
        assert!(
            value["consent_evidence"]
                .get("client_claims_digest")
                .is_none(),
            "bug: a grant without claims states a claims digest"
        );
        assert_eq!(
            value["approval"]["baseline"], true,
            "bug: a baseline is not marked"
        );
        Ok(())
    }

    #[test]
    fn a_terminal_event_without_an_issue_states_a_null_issue_receipt() -> AppResult<()> {
        let grant = grant(None, None)?;
        let (text, _) = GrantReceipt {
            receipt_id: "receipt-1",
            event: GrantEvent::Revoke,
            at: grant.granted_at,
            grant: &grant,
            approval: ReceiptApproval::by(GrantActor::Client),
            issue_receipt_id: None,
        }
        .payload()?;
        let value: Value = json(&text)?;
        assert!(
            value.get("issue_receipt_id").is_some_and(Value::is_null),
            "bug: a revoke without an issue does not state it as null: {text}"
        );
        Ok(())
    }

    #[test]
    fn a_grant_past_its_expiry_ends_as_expired_by_the_as() -> AppResult<()> {
        let expires_at = at(1_700_000_100)?;
        let grant = grant(None, Some(expires_at))?;
        let before = at(1_700_000_099)?;
        assert_eq!(
            GrantEvent::ending(&grant, GrantActor::User, before),
            (GrantEvent::Revoke, GrantActor::User, before),
            "bug: a live grant's ending is not its revoke"
        );
        let after = at(1_700_000_200)?;
        assert_eq!(
            GrantEvent::ending(&grant, GrantActor::User, after),
            (GrantEvent::Expire, GrantActor::Auto, expires_at),
            "bug: an expired grant's ending is not its expiry"
        );
        assert_eq!(
            GrantEvent::ending(&grant, GrantActor::Client, expires_at),
            (GrantEvent::Expire, GrantActor::Auto, expires_at),
            "bug: a grant ended at its expiry instant is not expired"
        );
        Ok(())
    }

    #[test]
    fn stored_events_and_actors_round_trip() -> AppResult<()> {
        for event in [GrantEvent::Issue, GrantEvent::Revoke, GrantEvent::Expire] {
            assert_eq!(GrantEvent::parse(event.as_str())?, event, "bug: event");
        }
        for actor in [GrantActor::User, GrantActor::Client, GrantActor::Auto] {
            assert_eq!(GrantActor::parse(actor.as_str())?, actor, "bug: actor");
        }
        assert!(
            GrantEvent::parse("renew").is_err(),
            "bug: an unknown event reads"
        );
        assert!(
            GrantActor::parse("admin").is_err(),
            "bug: an unknown actor reads"
        );
        Ok(())
    }
}
