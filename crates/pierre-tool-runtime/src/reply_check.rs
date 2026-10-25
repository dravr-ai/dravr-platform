// ABOUTME: The note a tool result carries when its caller writes its own reply — Dravr did not check that reply
// ABOUTME: Added at the executor to MCP and A2A results that served the athlete's data, naming verify_claim as the check
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where Dravr's claim checker reaches (carnet#852).
//!
//! The checker runs in the chat pipeline, on the replies Dravr's own agent
//! writes for the web app, the mobile app and messaging. An MCP client or an
//! A2A agent calls Dravr's tools and writes its own reply from what they
//! return, and that reply never reaches the checker. A result handed to such a
//! caller with the athlete's data in it says so in [`REPLY_CHECK_KEY`], a
//! field the caller can read without parsing prose, and names `verify_claim`,
//! the tool that checks a claim against the same athlete context the chat
//! pipeline uses.

use serde_json::{json, Map, Value};

use crate::ai_view::attach_notes;
use crate::protocol::types::UniversalResponse;

/// Key of the note on a result whose caller writes the reply Dravr does not
/// check. A wire value MCP clients and A2A agents parse.
const REPLY_CHECK_KEY: &str = "_reply_verification";

/// The tool a caller checks a claim with before stating it.
const VERIFY_WITH: &str = "verify_claim";

/// What the note says, for a reader that does read prose.
const UNCHECKED_REPLY_NOTE: &str = "Dravr checks the claims in replies it writes in its own app \
    and messaging. A reply you write from this result is not checked: call verify_claim on each \
    figure about the athlete (category athlete_data) before stating it.";

/// Mark a result whose caller writes its own reply from it: Dravr did not
/// check that reply.
pub(crate) fn note_unchecked_reply(response: &mut UniversalResponse) {
    let mut notes = Map::new();
    notes.insert(REPLY_CHECK_KEY.to_owned(), unchecked_reply_note());
    attach_notes(response, notes);
}

/// The note itself: `reply_checked` is the machine-readable answer.
fn unchecked_reply_note() -> Value {
    json!({
        "reply_checked": false,
        "verify_with": VERIFY_WITH,
        "note": UNCHECKED_REPLY_NOTE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(result: Option<Value>) -> UniversalResponse {
        UniversalResponse {
            success: true,
            result,
            error: None,
            metadata: None,
        }
    }

    #[test]
    fn the_note_rides_beside_what_the_tool_returned() {
        let mut served = response(Some(json!({ "activities": [{ "distance_km": 21.1 }] })));
        note_unchecked_reply(&mut served);

        let result = served.result.unwrap();
        assert_eq!(result["activities"][0]["distance_km"], 21.1);
        assert_eq!(result[REPLY_CHECK_KEY]["reply_checked"], false);
        assert_eq!(result[REPLY_CHECK_KEY]["verify_with"], "verify_claim");
        let note = result[REPLY_CHECK_KEY]["note"].as_str().unwrap();
        assert!(
            note.contains("own app and messaging") && note.contains("athlete_data"),
            "the note must scope the checker to Dravr's own surfaces and name the category: {note}"
        );
    }

    #[test]
    fn a_result_that_is_not_an_object_is_kept_under_result() {
        let mut served = response(Some(json!([1, 2, 3])));
        note_unchecked_reply(&mut served);

        let result = served.result.unwrap();
        assert_eq!(result["result"], json!([1, 2, 3]));
        assert_eq!(result[REPLY_CHECK_KEY]["reply_checked"], false);
    }
}
