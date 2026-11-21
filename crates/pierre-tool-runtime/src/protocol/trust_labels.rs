// ABOUTME: The trust label a tool result carries for a caller that writes its own reply: tier, source class, flags
// ABOUTME: Derived from the tool's declared streams and the source classes the call served (owl phase 1, T02 §3.1)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Trust labels on tool results (owl phase 1).
//!
//! A caller that writes its own reply from a result — an MCP client, an A2A
//! agent — is told how sensitive the data it was served is and how far to
//! trust it, in the result's `_meta` under [`TRUST_LABELS_META_KEY`]. The
//! label is for the host, never the model: the in-band `_reply_verification`
//! note beside it is the model's.
//!
//! - `tier` is the highest tier among the streams the tool declares; a tool
//!   that served an item and declares no stream is labelled T3, since an
//!   unplaced item takes the higher tier (owl §4.3).
//! - `source_class` is the class of every provider whose items the call
//!   served, plus `agent_written` for the agent's own memory or plan.
//! - `sensitive` is a tier of T2 or above (owl T02 rule 5).
//! - `untrusted` is a tool that passes provider-written text through, or a
//!   call that served another application's or the agent's own content (owl
//!   T02 rule 4).
//!
//! A call that served nothing carries no label: no label makes no claim (owl
//! T02 rule 3).

use std::collections::BTreeSet;

use dravr_barrage::Tier;
use pierre_core::ai_policy::SourceClass;
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use crate::protocol::UniversalResponse;
use crate::tool_data::ToolData;

/// The `_meta` key a tool result's trust label is carried under.
pub const TRUST_LABELS_META_KEY: &str = "ai.dravr.vault/labels";

/// The label's format version.
const LABEL_VERSION: u8 = 1;

/// One result's trust label, as it goes on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TrustLabels {
    /// The label format's version.
    v: u8,
    /// The most sensitive tier of what was served.
    tier: Tier,
    /// Where what was served came from.
    source_class: BTreeSet<SourceClass>,
    /// Whether the tier is T2 or above.
    sensitive: bool,
    /// Whether the result carries text or content no one at Dravr checked.
    untrusted: bool,
}

impl TrustLabels {
    /// The label of a call to a tool declaring `data` that served items of
    /// `served`; `None` when it served nothing.
    fn derive(
        tool_name: &str,
        data: Option<&ToolData>,
        served: &BTreeSet<SourceClass>,
    ) -> Option<Self> {
        if served.is_empty() {
            return None;
        }
        let tier = data.and_then(ToolData::tier).unwrap_or_else(|| {
            warn!(
                tool = tool_name,
                "a tool that declares no stream served an item; its result is labelled T3"
            );
            Tier::T3
        });
        let untrusted = data.is_some_and(ToolData::athlete_text)
            || served.contains(&SourceClass::ThirdPartyApp)
            || served.contains(&SourceClass::AgentWritten);
        Some(Self {
            v: LABEL_VERSION,
            tier,
            source_class: served.clone(),
            sensitive: tier >= Tier::T2,
            untrusted,
        })
    }
}

/// Put the trust label of a call to `tool_name`, which declares `data` and
/// served items of `served`, in `universal`'s metadata under
/// [`TRUST_LABELS_META_KEY`]. Nothing is added when the call served nothing.
pub(crate) fn attach(
    universal: &mut UniversalResponse,
    tool_name: &str,
    data: Option<&ToolData>,
    served: &BTreeSet<SourceClass>,
) {
    let Some(labels) = TrustLabels::derive(tool_name, data, served) else {
        return;
    };
    match serde_json::to_value(&labels) {
        Ok(value) => {
            universal
                .metadata
                .get_or_insert_with(Default::default)
                .insert(TRUST_LABELS_META_KEY.to_owned(), value);
        }
        Err(error) => warn!(tool = tool_name, %error, "trust label did not serialize"),
    }
}

/// The trust label `universal` carries, if any.
#[must_use]
pub fn labels_of(universal: &UniversalResponse) -> Option<&Value> {
    universal
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(TRUST_LABELS_META_KEY))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use dravr_barrage::Stream;
    use serde_json::json;

    use super::*;
    use crate::read_subject::DataClass;

    fn served(classes: &[SourceClass]) -> BTreeSet<SourceClass> {
        classes.iter().copied().collect()
    }

    fn label(data: Option<&ToolData>, classes: &[SourceClass]) -> Option<Value> {
        let mut universal = UniversalResponse {
            success: true,
            result: Some(json!({})),
            error: None,
            metadata: None,
        };
        attach(&mut universal, "tool", data, &served(classes));
        labels_of(&universal).cloned()
    }

    #[test]
    fn the_tier_is_the_most_sensitive_declared_stream() {
        let history = ToolData::of(
            DataClass::Training,
            &[Stream::TrainingLoad, Stream::Workout],
        );
        let activities = ToolData::of(
            DataClass::Training,
            &[Stream::Workout, Stream::WorkoutRoute],
        );
        assert_eq!(
            label(Some(&history), &[SourceClass::Device]),
            Some(json!({
                "v": 1, "tier": "T0", "source_class": ["device"],
                "sensitive": false, "untrusted": false
            })),
            "a load history from a device is T0 and trusted"
        );
        assert_eq!(
            label(Some(&activities), &[SourceClass::Device]).map(|l| l["tier"].clone()),
            Some(json!("T1")),
            "a route joins a workout at T1"
        );
    }

    #[test]
    fn a_tool_declaring_no_stream_is_labelled_t3_and_sensitive() {
        let undeclared = ToolData::of(DataClass::NoSubject, &[]);
        for data in [None, Some(&undeclared)] {
            let label = label(data, &[SourceClass::Device]);
            assert_eq!(
                label
                    .as_ref()
                    .map(|l| (l["tier"].clone(), l["sensitive"].clone())),
                Some((json!("T3"), json!(true))),
                "an unplaced item takes the higher tier"
            );
        }
    }

    #[test]
    fn sensitive_means_t2_and_above() {
        let notes = ToolData::own(&[Stream::RestrictedNotes]);
        let sleep = ToolData::of(DataClass::Wellness, &[Stream::Sleep]);
        assert_eq!(
            label(Some(&notes), &[SourceClass::AgentWritten]).map(|l| l["sensitive"].clone()),
            Some(json!(true)),
            "notes are T3"
        );
        assert_eq!(
            label(Some(&sleep), &[SourceClass::Device]).map(|l| l["sensitive"].clone()),
            Some(json!(false)),
            "T1 is not sensitive"
        );
    }

    #[test]
    fn untrusted_follows_written_text_and_the_source_class() {
        let plain = ToolData::of(DataClass::Training, &[Stream::Workout]);
        let text = plain.with_athlete_text();
        let untrusted = |data: &ToolData, classes: &[SourceClass]| {
            label(Some(data), classes).map(|l| l["untrusted"].clone())
        };
        assert_eq!(
            untrusted(&plain, &[SourceClass::Device]),
            Some(json!(false)),
            "a device's numbers with no written text are trusted"
        );
        assert_eq!(
            untrusted(&text, &[SourceClass::Device]),
            Some(json!(true)),
            "a tool passing provider text through is untrusted whatever served it"
        );
        assert_eq!(
            untrusted(&plain, &[SourceClass::Device, SourceClass::ThirdPartyApp]),
            Some(json!(true)),
            "another app's data is untrusted"
        );
        assert_eq!(
            untrusted(&plain, &[SourceClass::AgentWritten]),
            Some(json!(true)),
            "the agent's own writing is untrusted"
        );
        assert_eq!(
            untrusted(&plain, &[SourceClass::PatientEntered]),
            Some(json!(false)),
            "what the athlete typed in is theirs"
        );
    }

    #[test]
    fn a_call_that_served_nothing_carries_no_label() {
        let history = ToolData::of(DataClass::Training, &[Stream::TrainingLoad]);
        assert_eq!(label(Some(&history), &[]), None, "no label makes no claim");
    }

    #[test]
    fn a_label_keeps_the_metadata_already_there() {
        let mut universal = UniversalResponse {
            success: true,
            result: None,
            error: None,
            metadata: Some(HashMap::from([("other".to_owned(), json!(1))])),
        };
        attach(
            &mut universal,
            "tool",
            None,
            &served(&[SourceClass::Device]),
        );
        let metadata = universal.metadata.unwrap_or_default();
        assert_eq!(
            metadata.get("other"),
            Some(&json!(1)),
            "the other key stays"
        );
        assert!(
            metadata.contains_key(TRUST_LABELS_META_KEY),
            "the label joins it"
        );
    }
}
