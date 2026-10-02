// ABOUTME: What of a provider's data may enter a model prompt — per-(provider, source) rules declared by provider terms
// ABOUTME: Resolves the rules for an item and applies them to its JSON form or to typed items via a serde round trip

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider terms that keep data away from AI.
//!
//! Some provider terms let the athlete see their data but bar it from any AI
//! model (Nolio's API Annex for Strava, WHOOP, Zepp and Huawei connector data),
//! or bar only the provider's own proprietary scores (WHOOP). The rule belongs
//! to the provider, so it is declared next to the provider's descriptor as a
//! [`SourcePolicy`], and applied where data is handed to a model — never in
//! each tool.
//!
//! An item is governed by two rules, applied in order:
//! 1. the **relay's** rule for the item's upstream `source` — a coach platform
//!    that re-hosts a Strava activity decides what its terms allow for it;
//! 2. the **origin's** own direct rule, when the source is itself a provider
//!    Dravr knows — Strava's terms bind Strava data whichever service relayed it.
//!
//! The athlete's own surfaces never pass through here: the web and mobile apps
//! read the cache directly, so a withheld item stays visible to its owner.

use std::collections::BTreeSet;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

/// What one rule lets a model see of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiUse {
    /// Everything.
    Allow,
    /// Everything except these fields, which are removed.
    Redact(&'static [&'static str]),
    /// Only these fields: the item's existence (date, sport, duration), not
    /// its content.
    ExistenceOnly(&'static [&'static str]),
    /// Nothing; the item is dropped.
    Deny,
}

/// A provider's AI rules, declared by its terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePolicy {
    /// The rule for data the provider recorded itself.
    pub direct: AiUse,
    /// The rule per upstream connector, for a provider that relays data
    /// another service recorded (matched case-insensitively).
    pub by_source: &'static [(&'static str, AiUse)],
    /// The rule for a relayed source `by_source` does not name.
    pub other_sources: AiUse,
}

impl SourcePolicy {
    /// No restriction: the default for a provider whose terms set none.
    pub const ALLOW_ALL: Self = Self {
        direct: AiUse::Allow,
        by_source: &[],
        other_sources: AiUse::Allow,
    };

    /// The rule this policy sets for an item from `source`.
    #[must_use]
    pub fn rule_for(&self, provider: &str, source: Option<&str>) -> AiUse {
        match source {
            Some(src) if !src.eq_ignore_ascii_case(provider) => self
                .by_source
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(src))
                .map_or(self.other_sources, |(_, rule)| *rule),
            _ => self.direct,
        }
    }
}

/// Looks up a provider's AI policy by name. Implemented by the provider
/// registry; a provider it does not know has no restriction.
pub trait AiPolicyLookup: Send + Sync {
    /// The policy of `provider`, or `None` when the provider is unknown.
    fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy>;
}

/// The rules governing an item from `provider` whose upstream is `source`,
/// in application order. Empty when nothing restricts it.
#[must_use]
pub fn rules_for(lookup: &dyn AiPolicyLookup, provider: &str, source: Option<&str>) -> Vec<AiUse> {
    let mut rules = Vec::with_capacity(2);
    if let Some(policy) = lookup.ai_policy(provider) {
        rules.push(policy.rule_for(provider, source));
    }
    if let Some(src) = source.filter(|src| !src.eq_ignore_ascii_case(provider)) {
        let normalized = src.to_ascii_lowercase();
        if let Some(policy) = lookup.ai_policy(&normalized) {
            rules.push(policy.direct);
        }
    }
    rules.retain(|rule| *rule != AiUse::Allow);
    rules
}

/// What applying the rules did to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Unchanged.
    Kept,
    /// Some fields were removed.
    Reduced,
    /// The whole item was dropped.
    Dropped,
}

/// Apply `rules` to one item's JSON object, in order.
///
/// A non-object value is left as is when every rule keeps content, and
/// dropped when any rule denies it: a rule that cannot be honoured
/// field-by-field is honoured whole.
#[must_use]
pub fn apply_to_value(rules: &[AiUse], item: &mut Value) -> Outcome {
    let mut outcome = Outcome::Kept;
    for rule in rules {
        match rule {
            AiUse::Allow => {}
            AiUse::Deny => return Outcome::Dropped,
            AiUse::Redact(fields) => {
                let Some(object) = item.as_object_mut() else {
                    return Outcome::Dropped;
                };
                for field in *fields {
                    if object.remove(*field).is_some() {
                        outcome = Outcome::Reduced;
                    }
                }
            }
            AiUse::ExistenceOnly(keep) => {
                let Some(object) = item.as_object_mut() else {
                    return Outcome::Dropped;
                };
                let before = object.len();
                object.retain(|key, _| keep.contains(&key.as_str()));
                if object.len() != before {
                    outcome = Outcome::Reduced;
                }
            }
        }
    }
    outcome
}

/// A count of what the rules held back from the model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Withheld {
    /// Items dropped whole.
    pub dropped: usize,
    /// Items with some fields removed.
    pub reduced: usize,
    /// The upstream services the withheld data came from.
    pub sources: BTreeSet<String>,
}

impl Withheld {
    /// Whether anything was held back.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dropped == 0 && self.reduced == 0
    }

    /// Fold another count into this one.
    pub fn merge(&mut self, other: Self) {
        self.dropped += other.dropped;
        self.reduced += other.reduced;
        self.sources.extend(other.sources);
    }

    fn record(&mut self, outcome: Outcome, provider: &str, source: Option<&str>) {
        match outcome {
            Outcome::Kept => return,
            Outcome::Reduced => self.reduced += 1,
            Outcome::Dropped => self.dropped += 1,
        }
        self.sources
            .insert(source.unwrap_or(provider).to_ascii_lowercase());
    }

    /// The neutral note a model reads in place of what was withheld: enough
    /// to say that data exists it cannot see, never the data itself.
    #[must_use]
    pub fn to_note(&self) -> Value {
        serde_json::json!({
            "items_dropped": self.dropped,
            "items_reduced": self.reduced,
            "sources": self.sources.iter().collect::<Vec<_>>(),
            "reason": "provider_terms",
            "note": "Some of this athlete's data is withheld from AI by the terms of the service it came from. The athlete can still see it in the app; do not guess its contents.",
        })
    }
}

/// Filter typed items for a model.
///
/// `origin` names each item's provider and upstream source. An item no rule
/// restricts is passed through untouched, without a serde round trip. A
/// restricted item is serialized, reduced and deserialized back; `required`
/// lists the string fields the type cannot deserialize without, filled with
/// a neutral placeholder when a rule removed them. An item that still cannot
/// be rebuilt is dropped — a rule is never relaxed to keep an item typed.
pub fn filter_items<T, F>(
    lookup: &dyn AiPolicyLookup,
    items: Vec<T>,
    origin: F,
    required: &[&str],
) -> (Vec<T>, Withheld)
where
    T: Serialize + DeserializeOwned,
    F: Fn(&T) -> (String, Option<String>),
{
    let mut withheld = Withheld::default();
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        let (provider, source) = origin(&item);
        let rules = rules_for(lookup, &provider, source.as_deref());
        if rules.is_empty() {
            kept.push(item);
            continue;
        }
        let Ok(mut value) = serde_json::to_value(&item) else {
            withheld.record(Outcome::Dropped, &provider, source.as_deref());
            continue;
        };
        let outcome = apply_to_value(&rules, &mut value);
        if outcome == Outcome::Kept {
            kept.push(item);
            continue;
        }
        let rebuilt = (outcome == Outcome::Reduced)
            .then(|| fill_required(value, required))
            .and_then(|value| serde_json::from_value(value).ok());
        match rebuilt {
            Some(reduced) => {
                kept.push(reduced);
                withheld.record(outcome, &provider, source.as_deref());
            }
            None => withheld.record(Outcome::Dropped, &provider, source.as_deref()),
        }
    }
    (kept, withheld)
}

/// Placeholder for a required text field a rule removed.
pub const WITHHELD_PLACEHOLDER: &str = "(withheld by provider terms)";

fn fill_required(mut value: Value, required: &[&str]) -> Value {
    if let Some(object) = value.as_object_mut() {
        for field in required {
            object
                .entry((*field).to_owned())
                .or_insert_with(|| Value::String(WITHHELD_PLACEHOLDER.to_owned()));
        }
    }
    value
}

/// Filter every provider item found anywhere in a JSON tree, in place.
///
/// An item is any object carrying a string `provider` key (and optionally a
/// `source` key). Items are filtered where they sit: a dropped item is removed
/// from its array, or replaced by `null` as an object field. This is the
/// backstop at the tool output boundary, for structured data a read path did
/// not already filter.
pub fn filter_json(lookup: &dyn AiPolicyLookup, value: &mut Value) -> Withheld {
    let mut withheld = Withheld::default();
    filter_json_into(lookup, value, &mut withheld);
    withheld
}

fn item_origin(object: &Map<String, Value>) -> Option<(String, Option<String>)> {
    let provider = object.get("provider")?.as_str()?.to_owned();
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some((provider, source))
}

/// Applies the rules to `value` if it is an item; true when it was dropped.
fn filter_item(lookup: &dyn AiPolicyLookup, value: &mut Value, withheld: &mut Withheld) -> bool {
    let Some((provider, source)) = value.as_object().and_then(item_origin) else {
        return false;
    };
    let rules = rules_for(lookup, &provider, source.as_deref());
    if rules.is_empty() {
        return false;
    }
    let outcome = apply_to_value(&rules, value);
    withheld.record(outcome, &provider, source.as_deref());
    outcome == Outcome::Dropped
}

fn filter_json_into(lookup: &dyn AiPolicyLookup, value: &mut Value, withheld: &mut Withheld) {
    match value {
        Value::Array(items) => {
            items.retain_mut(|item| !filter_item(lookup, item, withheld));
            for item in items {
                filter_json_into(lookup, item, withheld);
            }
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                if filter_item(lookup, child, withheld) {
                    *child = Value::Null;
                } else {
                    filter_json_into(lookup, child, withheld);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    const NOLIO_KEEP: &[&str] = &["id", "provider", "source", "sport_type", "start_date"];

    const NOLIO: SourcePolicy = SourcePolicy {
        direct: AiUse::Allow,
        by_source: &[
            ("garmin", AiUse::Allow),
            ("strava", AiUse::ExistenceOnly(NOLIO_KEEP)),
            ("whoop", AiUse::ExistenceOnly(NOLIO_KEEP)),
            ("zepp", AiUse::Deny),
        ],
        other_sources: AiUse::Allow,
    };

    const WHOOP: SourcePolicy = SourcePolicy {
        direct: AiUse::Redact(&["recovery_score"]),
        by_source: &[],
        other_sources: AiUse::Allow,
    };

    struct Lookup;

    impl AiPolicyLookup for Lookup {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "nolio" => Some(&NOLIO),
                "whoop" => Some(&WHOOP),
                "garmin" | "strava" => Some(&SourcePolicy::ALLOW_ALL),
                _ => None,
            }
        }
    }

    #[test]
    fn relay_rule_then_origin_rule() {
        assert!(rules_for(&Lookup, "garmin", None).is_empty());
        assert!(rules_for(&Lookup, "nolio", Some("garmin")).is_empty());
        assert_eq!(rules_for(&Lookup, "nolio", Some("ZEPP")), vec![AiUse::Deny]);
        assert_eq!(
            rules_for(&Lookup, "nolio", Some("whoop")),
            vec![
                AiUse::ExistenceOnly(NOLIO_KEEP),
                AiUse::Redact(&["recovery_score"])
            ]
        );
        assert_eq!(
            rules_for(&Lookup, "whoop", Some("whoop")),
            vec![AiUse::Redact(&["recovery_score"])]
        );
        assert!(rules_for(&Lookup, "unknown", Some("other")).is_empty());
    }

    #[test]
    fn a_mixed_result_reaches_the_model_only_as_the_nolio_policy_permits() {
        let mut payload = json!({
            "activities": [
                {"id": "1", "provider": "nolio", "source": "garmin", "sport_type": "ride",
                 "start_date": "2026-09-30", "name": "Hills", "average_heart_rate": 150},
                {"id": "2", "provider": "nolio", "source": "strava", "sport_type": "run",
                 "start_date": "2026-09-29", "name": "Tempo", "average_heart_rate": 160},
                {"id": "3", "provider": "nolio", "source": "whoop", "sport_type": "row",
                 "start_date": "2026-09-28", "name": "Erg", "recovery_score": 34},
                {"id": "4", "provider": "nolio", "source": "zepp", "sport_type": "walk",
                 "start_date": "2026-09-27", "name": "Walk"}
            ],
            "count": 4
        });
        let withheld = filter_json(&Lookup, &mut payload);

        assert_eq!(
            payload["activities"],
            json!([
                {"id": "1", "provider": "nolio", "source": "garmin", "sport_type": "ride",
                 "start_date": "2026-09-30", "name": "Hills", "average_heart_rate": 150},
                {"id": "2", "provider": "nolio", "source": "strava", "sport_type": "run",
                 "start_date": "2026-09-29"},
                {"id": "3", "provider": "nolio", "source": "whoop", "sport_type": "row",
                 "start_date": "2026-09-28"}
            ])
        );
        assert_eq!(withheld.dropped, 1);
        assert_eq!(withheld.reduced, 2);
        assert_eq!(
            withheld
                .sources
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["strava", "whoop", "zepp"]
        );
    }

    #[test]
    fn a_dropped_object_field_becomes_null() {
        let mut payload = json!({"latest": {"provider": "nolio", "source": "zepp", "id": "9"}});
        let withheld = filter_json(&Lookup, &mut payload);
        assert_eq!(payload, json!({"latest": null}));
        assert_eq!(withheld.dropped, 1);
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Item {
        id: String,
        name: String,
        provider: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovery_score: Option<u32>,
    }

    fn item(id: &str, provider: &str, source: Option<&str>) -> Item {
        Item {
            id: id.to_owned(),
            name: "Session".to_owned(),
            provider: provider.to_owned(),
            source: source.map(str::to_owned),
            recovery_score: Some(40),
        }
    }

    #[test]
    fn typed_items_are_reduced_rebuilt_or_dropped() {
        let items = vec![
            item("a", "garmin", None),
            item("b", "whoop", None),
            item("c", "nolio", Some("strava")),
            item("d", "nolio", Some("zepp")),
        ];
        let (kept, withheld) = filter_items(
            &Lookup,
            items,
            |i| (i.provider.clone(), i.source.clone()),
            &["name"],
        );

        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0], item("a", "garmin", None));
        assert_eq!(kept[1].recovery_score, None, "WHOOP's score is redacted");
        assert_eq!(kept[1].name, "Session", "WHOOP's measurements stay");
        assert_eq!(kept[2].name, WITHHELD_PLACEHOLDER);
        assert_eq!(kept[2].recovery_score, None);
        assert_eq!(withheld.dropped, 1);
        assert_eq!(withheld.reduced, 2);
    }

    #[test]
    fn an_item_no_rule_touches_is_not_counted() {
        let (kept, withheld) = filter_items(
            &Lookup,
            vec![item("a", "strava", None)],
            |i| (i.provider.clone(), i.source.clone()),
            &["name"],
        );
        assert_eq!(kept.len(), 1);
        assert!(withheld.is_empty());
    }
}
