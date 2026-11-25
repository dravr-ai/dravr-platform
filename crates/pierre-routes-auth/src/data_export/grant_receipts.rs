// ABOUTME: The athlete's consent grant receipts in the export, projected as DPV ConsentRecords (owl PDPP §8.2)
// ABOUTME: One JSON-LD record per receipt, built from the signed payload, written just after the oauth_grant_events table

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Consent records (carnet#1544, design row 34, owl PDPP §8.2)
//!
//! The export carries the athlete's receipts as the `oauth_grant_events`
//! table, and beside it [`CONSENT_RECORDS_TABLE`]: one W3C DPV
//! `dpv:ConsentRecord` per receipt, the ISO/IEC TS 27560 projection owl's
//! §8.2 maps, so a tool that reads consent records reads the athlete's
//! grants without knowing Dravr's receipt format.
//!
//! Each record is built from the receipt's signed payload, never from the
//! grant row: a grant deleted with its client keeps its receipts, and the
//! record states what the receipt signed.
//!
//! | Receipt | DPV |
//! |---|---|
//! | `receipt_id` | `@id`, `urn:health-receipt:<id>` |
//! | the athlete | `dpv:hasDataSubject`, `urn:dravr:user:<user_id>` |
//! | `grant.client_id` | `dpv:hasRecipient` |
//! | `dpv_purpose` | `dpv:hasPurpose`, omitted when the client named none |
//! | the details' `selection_preset` | `dpv:hasPersonalData`, the preset's IRI; omitted for a plain grant |
//! | `event` | `dpv:hasConsentStatus`: Given, Withdrawn, Expired |
//! | `consent_evidence.review_digest` | `dpv:hasNotice` |
//! | the details' `retention` | `dpv:hasStorageCondition` |
//!
//! `hasPersonalData` names the preset the athlete chose, as the receipt
//! signed it, under `https://dravr.ai/pdpp/health-profile/v0/preset#`: the
//! stream names a preset reads are not published (E5). A record pairs with
//! its `record_log` entry through its receipt: the entry's content is the
//! receipt's `payload_sha256`, in the `oauth_grant_events` row of the same
//! id.

use pierre_core::models::{details_member, GrantEvent, OAuthGrantEvent};
use serde_json::{json, Map, Value};
use tracing::warn;
use uuid::Uuid;

use super::barrage::ExportSources;
use super::{ChunkSink, ExportSummary, Writer};

/// The section the consent records are written as.
pub(super) const CONSENT_RECORDS_TABLE: &str = "consent_records";

/// The platform table the consent records come just after: the receipts they
/// project.
pub(super) const CONSENT_RECORDS_AFTER: &str = "oauth_grant_events";

/// Receipts read per page.
const RECEIPT_PAGE: u32 = 100;

/// The namespace a preset's IRI is minted under.
const PRESET_IRI: &str = "https://dravr.ai/pdpp/health-profile/v0/preset#";

/// The DPV consent status a receipt's event states.
const fn consent_status(event: GrantEvent) -> &'static str {
    match event {
        GrantEvent::Issue => "dpv:ConsentGiven",
        GrantEvent::Revoke => "dpv:ConsentWithdrawn",
        GrantEvent::Expire => "dpv:ConsentExpired",
    }
}

/// `{"@id": iri}`.
fn node(iri: impl Into<String>) -> Value {
    json!({ "@id": iri.into() })
}

/// The DPV consent record of `receipt`, held by `user_id`.
///
/// # Errors
/// The payload's parse error when the stored receipt is not JSON.
pub(super) fn consent_record(
    receipt: &OAuthGrantEvent,
    user_id: Uuid,
) -> Result<Value, serde_json::Error> {
    let payload: Value = serde_json::from_str(&receipt.payload)?;
    let grant = &payload["grant"];
    let details = &grant["authorization_details"];
    let mut record = Map::new();
    record.insert(
        "@context".to_owned(),
        json!({ "dpv": "https://w3id.org/dpv#", "xsd": "http://www.w3.org/2001/XMLSchema#" }),
    );
    record.insert(
        "@id".to_owned(),
        Value::String(format!("urn:health-receipt:{}", receipt.id)),
    );
    record.insert("@type".to_owned(), Value::from("dpv:ConsentRecord"));
    record.insert(
        "dpv:hasDataSubject".to_owned(),
        node(format!("urn:dravr:user:{user_id}")),
    );
    if let Some(client_id) = grant["client_id"].as_str() {
        record.insert("dpv:hasRecipient".to_owned(), node(client_id));
    }
    if let Some(purpose) = payload["dpv_purpose"].as_str() {
        record.insert("dpv:hasPurpose".to_owned(), node(purpose));
    }
    if let Some(preset) = details_member(details, "selection_preset").and_then(Value::as_str) {
        record.insert(
            "dpv:hasPersonalData".to_owned(),
            json!([node(format!("{PRESET_IRI}{preset}"))]),
        );
    }
    record.insert(
        "dpv:hasConsentStatus".to_owned(),
        node(consent_status(receipt.event)),
    );
    if let Some(digest) = payload["consent_evidence"]["review_digest"].as_str() {
        record.insert(
            "dpv:hasNotice".to_owned(),
            node(format!("urn:dravr:review-digest:{digest}")),
        );
    }
    if let Some(duration) = details_member(details, "retention")
        .and_then(|retention| retention.get("max_duration"))
        .and_then(Value::as_str)
    {
        record.insert(
            "dpv:hasStorageCondition".to_owned(),
            json!({ "dpv:hasDuration": { "@value": duration, "@type": "xsd:duration" } }),
        );
    }
    Ok(Value::Object(record))
}

/// Write each of `page` as a consent record, comma-separated after any
/// written before (`first` tracks that). `false` once the export ran out of
/// room: the array is closed and the table recorded as skipped.
async fn write_receipts<S: ChunkSink>(
    page: &[OAuthGrantEvent],
    user_id: Uuid,
    writer: &mut Writer<'_, S>,
    summary: &mut ExportSummary,
    first: &mut bool,
) -> bool {
    let name = CONSENT_RECORDS_TABLE;
    for receipt in page {
        if let Some(limit) = writer.out_of_room() {
            summary.limit_reached = Some(limit);
            summary.skipped_tables.push(name);
            writer.put(b"]").await;
            return false;
        }
        if !*first {
            writer.put(b",").await;
        }
        *first = false;
        let written =
            consent_record(receipt, user_id).and_then(|record| serde_json::to_vec(&record));
        match written {
            Ok(text) => writer.put(&text).await,
            Err(error) => {
                warn!(table = name, receipt_id = %receipt.id, error = %error, "User export could not write a consent record");
                summary.mark_failed(name);
                writer.put(b"null").await;
            }
        }
        summary.rows += 1;
    }
    true
}

/// Write `"consent_records":[record,…]`: every receipt of the athlete, in
/// each tenant in turn, a page at a time.
pub(super) async fn write_consent_records<S: ChunkSink>(
    sources: &ExportSources<'_>,
    user_id: Uuid,
    writer: &mut Writer<'_, S>,
    summary: &mut ExportSummary,
) {
    let name = CONSENT_RECORDS_TABLE;
    writer.put(format!("\"{name}\":[").as_bytes()).await;
    let owner = user_id.to_string();
    let mut first = true;
    for tenant in sources.tenants {
        let tenant_id = tenant.to_string();
        let mut offset = 0_u32;
        loop {
            if writer.disconnected {
                return;
            }
            let page = match sources
                .grant_events
                .list_for_user(&tenant_id, &owner, None, RECEIPT_PAGE, offset)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    warn!(table = name, error = %error, "User export could not read a table");
                    summary.mark_failed(name);
                    break;
                }
            };
            if !write_receipts(&page, user_id, writer, summary, &mut first).await {
                return;
            }
            let full = u32::try_from(page.len()).is_ok_and(|read| read >= RECEIPT_PAGE);
            if !full {
                break;
            }
            offset = offset.saturating_add(RECEIPT_PAGE);
        }
    }
    writer.put(b"]").await;
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_core::models::GrantActor;

    use super::*;

    fn receipt(event: GrantEvent, payload: &Value) -> OAuthGrantEvent {
        OAuthGrantEvent {
            id: "receipt-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            user_id: "user-1".to_owned(),
            client_grant_id: "grant-1".to_owned(),
            event,
            actor: GrantActor::User,
            at: Timestamp::now(),
            payload: payload.to_string(),
            payload_sha256: "00".to_owned(),
            jws: None,
            logged_at: None,
        }
    }

    #[test]
    fn a_preset_grant_projects_every_dpv_member_it_carries() {
        let payload = json!({
            "grant": {
                "client_id": "https://coach.example/client.json",
                "authorization_details": [
                    {"type": "https://pdpp.dev/data-access", "selection_preset": "sleep_coach",
                     "retention": {"max_duration": "P90D", "on_expiry": "delete"}},
                    {"type": "https://dravr.ai/pdpp/health-profile/v0/health-constraints"}
                ]
            },
            "consent_evidence": {"review_digest": "abc123"},
            "dpv_purpose": "https://w3id.org/dpv#ServiceProvision"
        });
        let user = Uuid::nil();
        let record = consent_record(&receipt(GrantEvent::Revoke, &payload), user).unwrap();
        assert_eq!(
            record,
            json!({
                "@context": {"dpv": "https://w3id.org/dpv#", "xsd": "http://www.w3.org/2001/XMLSchema#"},
                "@id": "urn:health-receipt:receipt-1",
                "@type": "dpv:ConsentRecord",
                "dpv:hasDataSubject": {"@id": format!("urn:dravr:user:{user}")},
                "dpv:hasRecipient": {"@id": "https://coach.example/client.json"},
                "dpv:hasPurpose": {"@id": "https://w3id.org/dpv#ServiceProvision"},
                "dpv:hasPersonalData": [{"@id": "https://dravr.ai/pdpp/health-profile/v0/preset#sleep_coach"}],
                "dpv:hasConsentStatus": {"@id": "dpv:ConsentWithdrawn"},
                "dpv:hasNotice": {"@id": "urn:dravr:review-digest:abc123"},
                "dpv:hasStorageCondition": {"dpv:hasDuration": {"@value": "P90D", "@type": "xsd:duration"}}
            }),
            "bug: a preset receipt's record lost or renamed a member"
        );
    }

    #[test]
    fn a_plain_grant_names_no_data_purpose_or_storage() {
        let payload = json!({
            "grant": {"client_id": "client-1", "authorization_details": null},
            "consent_evidence": {"review_digest": null}
        });
        let record = consent_record(&receipt(GrantEvent::Expire, &payload), Uuid::nil()).unwrap();
        for absent in [
            "dpv:hasPurpose",
            "dpv:hasPersonalData",
            "dpv:hasNotice",
            "dpv:hasStorageCondition",
        ] {
            assert!(
                record.get(absent).is_none(),
                "bug: a plain grant's record states {absent}: {record}"
            );
        }
        assert_eq!(
            record["dpv:hasConsentStatus"]["@id"], "dpv:ConsentExpired",
            "bug: an expire receipt is not an expired consent"
        );
        assert_eq!(
            consent_record(&receipt(GrantEvent::Issue, &payload), Uuid::nil()).unwrap()
                ["dpv:hasConsentStatus"]["@id"],
            "dpv:ConsentGiven",
            "bug: an issue receipt is not a given consent"
        );
    }

    #[test]
    fn a_receipt_that_is_not_json_is_an_error() {
        let mut broken = receipt(GrantEvent::Issue, &Value::Null);
        broken.payload = "{not json".to_owned();
        assert!(
            consent_record(&broken, Uuid::nil()).is_err(),
            "bug: an unreadable receipt projected a record"
        );
    }
}
