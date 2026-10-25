// ABOUTME: Fits a tool reply under the token budget MCP clients enforce, trimming its largest list
// ABOUTME: A trimmed reply says so in a reply_budget sidecar, so the model knows the list is partial and how to page
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The reply budget.
//!
//! Claude Code refuses a tool reply above `MAX_MCP_OUTPUT_TOKENS` (25,000 by
//! default) and drops an oversized `structuredContent` outright, so a long
//! activity window reached the model as nothing at all. A reply over
//! [`REPLY_TOKEN_BUDGET`] keeps the head of its largest list, as many items as
//! fit, and says what it left out in a [`REPLY_BUDGET_KEY`] sidecar.
//!
//! The size is [`estimate_context_tokens`], the estimator that never
//! under-counts JSON. Measured against Anthropic's tokenizer on 2026-10-20 it
//! over-counted a 60-activity window by 8% and a `tools/list` slice by 34%,
//! so the budget below holds with room to spare.

use pierre_core::tokens::estimate_context_tokens;
use std::mem;

use serde_json::{json, Map, Value};
use tracing::warn;

/// The most tokens a tool reply may estimate at: a fifth under Claude Code's
/// 25,000 default, for the estimator's error and the protocol envelope.
pub(crate) const REPLY_TOKEN_BUDGET: u32 = 20_000;

/// The sidecar a trimmed reply carries.
pub(crate) const REPLY_BUDGET_KEY: &str = "reply_budget";

/// Where the list a reply is trimmed in sits: a top-level key, or a key of an
/// object under one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListPath {
    outer: String,
    inner: Option<String>,
}

impl ListPath {
    fn label(&self) -> String {
        self.inner.as_ref().map_or_else(
            || self.outer.clone(),
            |inner| format!("{}.{inner}", self.outer),
        )
    }

    fn get_mut<'a>(&self, object: &'a mut Map<String, Value>) -> Option<&'a mut Vec<Value>> {
        let outer = object.get_mut(&self.outer)?;
        let list = match &self.inner {
            None => outer,
            Some(inner) => outer.as_object_mut()?.get_mut(inner)?,
        };
        list.as_array_mut()
    }
}

/// `reply` as it may be sent: unchanged when it fits [`REPLY_TOKEN_BUDGET`],
/// otherwise with its largest list cut to the items that fit and a
/// [`REPLY_BUDGET_KEY`] sidecar naming the list, how many items it kept, and
/// how many there were.
///
/// A reply with no list to cut, or still over budget with that list emptied,
/// is sent as it is and logged: there is nothing in it to drop that the model
/// could page back to.
#[must_use]
pub(crate) fn fit_reply(reply: Value) -> Value {
    fit_reply_within(reply, REPLY_TOKEN_BUDGET)
}

fn fit_reply_within(reply: Value, budget: u32) -> Value {
    if fits(&reply, budget) {
        return reply;
    }
    let mut object = match reply {
        Value::Object(object) => object,
        // A bare list has no room for a sidecar; it is wrapped so it can say
        // it was cut.
        Value::Array(items) => Map::from_iter([("items".to_owned(), Value::Array(items))]),
        other => {
            warn_unfit(&other, "no list to trim");
            return other;
        }
    };
    let Some(path) = largest_list(&object) else {
        let reply = Value::Object(object);
        warn_unfit(&reply, "no list to trim");
        return reply;
    };
    let Some(full) = path.get_mut(&mut object).map(mem::take) else {
        return Value::Object(object);
    };
    let total = full.len();

    // The most leading items that fit, by bisection: each probe serializes
    // the whole reply, so a 500-item window costs nine probes, not 500.
    let (mut fit, mut over) = (0, total);
    while fit < over {
        let probe = (fit + over).div_ceil(2);
        if fits(&trimmed(&object, &path, &full, probe), budget) {
            fit = probe;
        } else {
            over = probe - 1;
        }
    }
    let reply = trimmed(&object, &path, &full, fit);
    if !fits(&reply, budget) {
        warn_unfit(&reply, "over budget with its largest list emptied");
    }
    reply
}

/// `object` with the list at `path` holding the first `keep` of `full`, and
/// the sidecar saying so.
fn trimmed(object: &Map<String, Value>, path: &ListPath, full: &[Value], keep: usize) -> Value {
    let mut object = object.clone();
    if let Some(list) = path.get_mut(&mut object) {
        *list = full[..keep].to_vec();
    }
    object.insert(
        REPLY_BUDGET_KEY.to_owned(),
        json!({
            "truncated": path.label(),
            "returned": keep,
            "total": full.len(),
            "note": format!(
                "Only the first {keep} of {total} items of `{label}` fit in one reply. \
                 Ask for fewer at a time (a smaller limit, a narrower date range) or page \
                 with offset to read the rest.",
                total = full.len(),
                label = path.label(),
            ),
        }),
    );
    Value::Object(object)
}

/// The list in `object` (top level, or one object down) whose items
/// serialize largest.
fn largest_list(object: &Map<String, Value>) -> Option<ListPath> {
    let mut lists = Vec::new();
    for (outer, value) in object {
        match value {
            Value::Array(items) => lists.push((
                ListPath {
                    outer: outer.clone(),
                    inner: None,
                },
                items,
            )),
            Value::Object(nested) => {
                for (inner, value) in nested {
                    if let Value::Array(items) = value {
                        lists.push((
                            ListPath {
                                outer: outer.clone(),
                                inner: Some(inner.clone()),
                            },
                            items,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    lists
        .into_iter()
        .filter(|(_, items)| !items.is_empty())
        .max_by_key(|(_, items)| items.iter().map(serialized_len).sum::<usize>())
        .map(|(path, _)| path)
}

fn fits(reply: &Value, budget: u32) -> bool {
    estimate_context_tokens(&reply.to_string()) <= budget
}

fn serialized_len(value: &Value) -> usize {
    value.to_string().len()
}

fn warn_unfit(reply: &Value, why: &str) {
    warn!(
        estimated_tokens = estimate_context_tokens(&reply.to_string()),
        budget = REPLY_TOKEN_BUDGET,
        why,
        "tool reply exceeds the reply budget"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activities(count: usize) -> Vec<Value> {
        (0..count)
            .map(|i| {
                json!({
                    "id": format!("act-{i:05}"),
                    "name": "Threshold intervals 5x1km",
                    "sport_type": "Run",
                    "distance_meters": 12_345.6,
                    "duration_seconds": 3_600,
                })
            })
            .collect()
    }

    #[test]
    fn a_reply_that_fits_is_unchanged() {
        let reply = json!({ "activities": activities(5), "count": 5 });
        assert_eq!(fit_reply(reply.clone()), reply);
    }

    #[test]
    fn an_oversized_list_keeps_its_head_and_says_so() {
        let reply = json!({ "activities": activities(400), "count": 400, "provider": "strava" });
        let fitted = fit_reply_within(reply, 2_000);
        let kept = fitted["activities"].as_array().unwrap();
        assert!(!kept.is_empty() && kept.len() < 400, "{}", kept.len());
        assert_eq!(kept[0]["id"], "act-00000");
        assert!(fits(&fitted, 2_000));
        let sidecar = &fitted[REPLY_BUDGET_KEY];
        assert_eq!(sidecar["truncated"], "activities");
        assert_eq!(sidecar["returned"], kept.len());
        assert_eq!(sidecar["total"], 400);
        assert_eq!(fitted["provider"], "strava");
    }

    #[test]
    fn the_largest_list_is_trimmed_even_one_object_down() {
        let reply = json!({
            "summary": { "weeks": activities(400) },
            "tags": ["a", "b"],
        });
        let fitted = fit_reply_within(reply, 2_000);
        assert_eq!(fitted[REPLY_BUDGET_KEY]["truncated"], "summary.weeks");
        assert_eq!(fitted["tags"], json!(["a", "b"]));
    }

    #[test]
    fn a_bare_list_is_wrapped_so_it_can_carry_the_sidecar() {
        let fitted = fit_reply_within(Value::Array(activities(400)), 2_000);
        assert_eq!(fitted[REPLY_BUDGET_KEY]["truncated"], "items");
    }

    #[test]
    fn a_reply_with_nothing_to_trim_is_sent_as_it_is() {
        let reply = json!({ "report": "x".repeat(200_000) });
        assert_eq!(fit_reply_within(reply.clone(), 2_000), reply);
    }
}
