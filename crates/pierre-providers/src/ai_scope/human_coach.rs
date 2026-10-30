// ABOUTME: A human coach's read of an athlete: only the athlete's withdrawable AI consents apply, never the model rules
// ABOUTME: The scope the roster REST read runs under, so it and the agent's tool honour one consent (carnet#1262)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A human coach reading an athlete who shares with them (carnet#1262).
//!
//! The coach's roster has two readers: the agent's `get_roster_overview`,
//! a model, and `GET /api/coach/roster`, a human. JF decided on 2026-10-30
//! (ADR-002 amendment) that both honour the athlete's withdrawable consent to
//! AI use of a provider's data, so the two stay one projection: a provider the
//! athlete withheld from AI is absent from the coach's screen too.
//!
//! Only that consent applies. The provider terms' AI rules (WHOOP's score
//! redaction, a Nolio relay's existence-only sources) govern what a *model*
//! may use; they are not terms on what a human may read, so a human coach
//! sees those items whole. Inside [`for_human_coach`] every filter point runs
//! as for a model, over terms that answer no AI rule for any provider —
//! [`ConsentOnly`] — on which the athlete's consent then lays
//! [`SourcePolicy::DENY_ALL`] for each provider withheld. The transport gate
//! is untouched. A model read nested inside keeps its full rules.

use std::future::Future;

use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
use pierre_core::transport::TransportPolicy;

tokio::task_local! {
    /// Set while a human coach reads an athlete who shares with them.
    static HUMAN_COACH: bool;
}

/// Run `fut` as a human coach's read of their athletes' data.
///
/// Inside it, a provider the athlete has not consented to hand to AI
/// ([`with_ai_consent`](super::with_ai_consent)) is withheld whole, as from a
/// model; every other item is served as it is. Pair it with the athlete's
/// declared consent — an undeclared one withholds every provider whose notice
/// is a consent to AI use.
pub fn for_human_coach<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    HUMAN_COACH.scope(true, fut)
}

/// Whether the read made now is a human coach's.
pub(super) fn human_coach_read() -> bool {
    HUMAN_COACH.try_with(|set| *set).unwrap_or(false)
}

/// A provider's terms with every AI rule lifted and its transport terms kept:
/// what a human coach's read filters through before the athlete's consent.
pub(super) struct ConsentOnly<'terms>(pub(super) &'terms dyn ProviderTerms);

impl ProviderTerms for ConsentOnly<'_> {
    fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
        None
    }

    fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
        self.0.transport_policy(provider)
    }

    fn item_transport_policy(
        &self,
        provider: &str,
        source: Option<&str>,
    ) -> Option<TransportPolicy> {
        self.0.item_transport_policy(provider, source)
    }

    fn relayed_transport_policy(&self, relay: &str, source: &str) -> Option<TransportPolicy> {
        self.0.relayed_transport_policy(relay, source)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{json, Value};

    use super::super::{ai_read, filter_typed, with_ai_consent};
    use super::*;
    use crate::provider_terms::{NOLIO, WHOOP};

    /// WHOOP and Nolio as their terms declare them for a model.
    struct Terms;

    impl ProviderTerms for Terms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "whoop" => Some(&WHOOP),
                "nolio" => Some(&NOLIO),
                _ => None,
            }
        }

        fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
            None
        }
    }

    fn whoop_morning() -> Value {
        json!({"id": "w", "provider": "whoop", "name": "Morning", "recovery_score": 71, "hrv": 64})
    }

    fn nolio_relay(source: &str) -> Value {
        json!({"id": source, "provider": "nolio", "source": source, "name": "Hills",
               "sport_type": "Ride", "average_heart_rate": 150})
    }

    fn filter(items: Vec<Value>) -> Vec<Value> {
        filter_typed(
            &Terms,
            items,
            |item: &Value| {
                (
                    item["provider"].as_str().unwrap_or_default().to_owned(),
                    item["source"].as_str().map(str::to_owned),
                )
            },
            &["name"],
        )
    }

    fn withheld(providers: &[&str]) -> BTreeSet<String> {
        providers.iter().map(|p| (*p).to_owned()).collect()
    }

    fn ids(items: &[Value]) -> Vec<&str> {
        items.iter().filter_map(|i| i["id"].as_str()).collect()
    }

    #[tokio::test]
    async fn a_provider_withheld_from_ai_is_withheld_from_the_coach_recorded_or_relayed() {
        let items = vec![whoop_morning(), nolio_relay("whoop"), nolio_relay("garmin")];
        let kept = for_human_coach(with_ai_consent(withheld(&["whoop"]), async {
            filter(items)
        }))
        .await;
        assert_eq!(ids(&kept), vec!["garmin"], "{kept:?}");
        assert_eq!(kept[0], nolio_relay("garmin"), "the rest is served whole");
    }

    #[tokio::test]
    async fn a_consented_whoop_record_keeps_its_scores_for_the_coach() {
        let kept = for_human_coach(with_ai_consent(BTreeSet::new(), async {
            filter(vec![whoop_morning()])
        }))
        .await;
        assert_eq!(
            kept,
            vec![whoop_morning()],
            "WHOOP's score redaction is a rule for a model, not for a human"
        );
    }

    #[tokio::test]
    async fn a_relayed_session_is_served_whole_to_the_coach() {
        let items = vec![nolio_relay("strava"), nolio_relay("zepp")];
        let kept = for_human_coach(with_ai_consent(BTreeSet::new(), async {
            filter(items.clone())
        }))
        .await;
        assert_eq!(
            kept, items,
            "a relay's existence-only and denied sources bind a model only"
        );
    }

    #[tokio::test]
    async fn a_model_read_inside_a_coachs_read_keeps_every_model_rule() {
        let items = vec![whoop_morning(), nolio_relay("strava"), nolio_relay("zepp")];
        let (kept, _) = for_human_coach(with_ai_consent(BTreeSet::new(), async {
            ai_read(async { filter(items) }).await
        }))
        .await;
        assert_eq!(ids(&kept), vec!["w", "strava"], "zepp is denied to a model");
        assert!(kept[0].get("recovery_score").is_none(), "{kept:?}");
        assert_eq!(kept[0]["hrv"], 64, "WHOOP's measurements stay");
        assert!(
            kept[1].get("average_heart_rate").is_none(),
            "strava via Nolio: existence only, {kept:?}"
        );
    }

    #[tokio::test]
    async fn outside_a_coachs_read_a_route_is_not_filtered_by_consent() {
        let kept = with_ai_consent(withheld(&["whoop"]), async {
            filter(vec![whoop_morning()])
        })
        .await;
        assert_eq!(
            kept,
            vec![whoop_morning()],
            "the athlete's own routes see their data"
        );
    }
}
