// ABOUTME: Parses Meta WhatsApp value.statuses[] delivery receipts + masks the recipient
// ABOUTME: Backs delivery visibility — a failed async push is logged, not a silent message_count=0
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests the WhatsApp delivery-status parser that makes a failed outbound push
//! (backfill-ready notice, reconnect nudge) observable instead of silent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_mcp_server::routes::messaging::webhooks::{
    mask_recipient, parse_whatsapp_delivery_statuses,
};

#[test]
fn parses_a_failed_delivery_with_error_code() {
    let body = br#"{
        "object": "whatsapp_business_account",
        "entry": [{
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "statuses": [{
                        "id": "wamid.HBgLABCDEF",
                        "status": "failed",
                        "timestamp": "1782850000",
                        "recipient_id": "14502244753",
                        "errors": [{
                            "code": 131047,
                            "title": "Re-engagement message",
                            "message": "Outside the 24h window"
                        }]
                    }]
                }
            }]
        }]
    }"#;

    let statuses = parse_whatsapp_delivery_statuses(body);
    assert_eq!(statuses.len(), 1);
    let s = &statuses[0];
    assert_eq!(s.status, "failed");
    assert_eq!(s.recipient_id, "14502244753");
    assert_eq!(s.message_id, "wamid.HBgLABCDEF");
    assert_eq!(s.error_code, Some(131_047));
    assert_eq!(s.error_title.as_deref(), Some("Re-engagement message"));
}

#[test]
fn parses_a_delivered_status_without_error() {
    let body = br#"{"entry":[{"changes":[{"value":{"statuses":[
        {"id":"wamid.X","status":"delivered","recipient_id":"14502244753"}
    ]}}]}]}"#;
    let statuses = parse_whatsapp_delivery_statuses(body);
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].status, "delivered");
    assert_eq!(statuses[0].error_code, None);
    assert!(statuses[0].error_title.is_none());
}

#[test]
fn inbound_message_webhook_yields_no_statuses() {
    // A normal inbound user message carries `value.messages[]`, not `statuses[]`.
    let body = br#"{"entry":[{"changes":[{"value":{"messages":[
        {"id":"wamid.in","from":"14502244753","text":{"body":"salut"}}
    ]}}]}]}"#;
    assert!(parse_whatsapp_delivery_statuses(body).is_empty());
}

#[test]
fn unparseable_body_yields_no_statuses() {
    assert!(parse_whatsapp_delivery_statuses(b"not json").is_empty());
    assert!(parse_whatsapp_delivery_statuses(b"{}").is_empty());
}

#[test]
fn mask_recipient_keeps_only_last_four() {
    assert_eq!(mask_recipient("14502244753"), "*******4753");
    assert_eq!(mask_recipient("123"), "****");
    assert_eq!(mask_recipient(""), "****");
}
