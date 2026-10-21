// ABOUTME: Pins the sleep tools' payload conversion: an f32 keeps its precision
// ABOUTME: and a payload that cannot serialize fails naming the tool that built it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `suggest_rest_day` and `optimize_sleep_schedule` answer through
//! `payload_value`, which serializes the way every reader-facing payload does
//! (carnet#532) and maps a failure to the protocol error their callers expect.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeMap;

use pierre_tool_runtime::implementations::sleep::output::payload_value;
use pierre_tool_runtime::protocols::ProtocolError;
use serde::Serialize;

#[derive(Serialize)]
struct Reading {
    hours: f32,
}

#[test]
fn an_f32_reads_as_written_not_widened() {
    let value = payload_value("suggest_rest_day", &Reading { hours: 7.3 }).unwrap();
    assert_eq!(value.to_string(), r#"{"hours":7.3}"#);
}

#[test]
fn a_payload_that_cannot_serialize_names_the_tool() {
    let tuple_keys: BTreeMap<(u8, u8), u8> = BTreeMap::from([((1, 2), 3)]);
    match payload_value("optimize_sleep_schedule", &tuple_keys) {
        Err(ProtocolError::InternalError(message)) => {
            assert!(
                message.starts_with("optimize_sleep_schedule: "),
                "{message}"
            );
        }
        other => panic!("expected InternalError, got {other:?}"),
    }
}
