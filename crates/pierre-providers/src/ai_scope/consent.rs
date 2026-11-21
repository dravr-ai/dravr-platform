// ABOUTME: The athlete's consent to AI use of each provider's data, as the AI-read scope applies it
// ABOUTME: A provider without it is denied whole to every model read, recorded or relayed; undeclared withholds (carnet#726)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Withdrawable AI consent (carnet#726).
//!
//! A provider whose notice is a consent to AI use (WHOOP's owner
//! authorization) can be withdrawn by the athlete at any time. Each entry
//! point that reads for a model on an athlete's behalf declares which
//! providers lack that consent ([`with_ai_consent`]); the parent module's
//! filters then read the provider terms through
//! [`WithConsent`](pierre_core::ai_policy::WithConsent), so such a provider's
//! data is denied whole to the model. A human coach's read of the athlete
//! ([`for_human_coach`](super::for_human_coach)) is held to the same consent
//! (carnet#1262). The athlete still sees it, and the connection stays live.
//! The parent module re-exports the public surface.
//!
//! The consents are always those of the person whose data is read. A call
//! that reads someone other than its caller — a human coach's agent reading
//! the athlete they coach — runs that read under the athlete's consents
//! ([`with_subject_consent`], carnet#1419), and the call's output boundary
//! applies the same ones ([`subject_consents`]): the caller's consents say
//! nothing about the athlete's providers, in either direction.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use dravr_barrage::Tier;
use pierre_core::ai_policy::Withheld;
use pierre_core::constants::oauth_providers::{
    ai_consent_backends, is_consent_class, SENSITIVE_T2,
};
use pierre_core::errors::{AppError, AppResult};

use super::{exposure, record_withheld};

tokio::task_local! {
    /// The providers whose data the athlete has not consented to hand to AI,
    /// declared at the entry point that knows the athlete.
    static AI_CONSENT_WITHHELD: Arc<BTreeSet<String>>;
    /// What the people a call read other than its caller withheld from AI
    /// ([`with_subject_consent`]), collected for the call's output boundary
    /// ([`subject_consents`]). `None` until the call reads such a person.
    /// Shared because the scope owns its value: [`subject_consents`] keeps a
    /// second handle to read what the call recorded once that scope closes.
    static SUBJECT_CONSENTS: Arc<Mutex<Option<BTreeSet<String>>>>;
}

/// `withheld` with every provider named as the filters compare it.
fn lowercased(withheld: BTreeSet<String>) -> BTreeSet<String> {
    withheld
        .into_iter()
        .map(|provider| provider.to_ascii_lowercase())
        .collect()
}

/// Run `fut` with `withheld` as the providers lacking the athlete's AI consent.
///
/// Those are the providers whose data the athlete has not consented to hand
/// to AI, or withdrew that consent for. Inside a read for a model or a human
/// coach's read ([`for_human_coach`](super::for_human_coach)), a provider in
/// it is denied whole, recorded or relayed; outside both it changes nothing.
/// Such a read where nothing was declared withholds every provider
/// whose notice is a consent to AI use ([`ai_consent_backends`]): an entry
/// point that forgets withholds the data instead of handing it over.
pub async fn with_ai_consent<F: Future>(withheld: BTreeSet<String>, fut: F) -> F::Output {
    AI_CONSENT_WITHHELD
        .scope(Arc::new(lowercased(withheld)), fut)
        .await
}

/// Run `fut`, a read of another person's data made for its caller — a human
/// coach's agent reading the athlete they coach — under that person's AI
/// consents, `withheld`.
///
/// Inside `fut` this is [`with_ai_consent`]: the subject's consents decide,
/// never the caller's, so a provider the subject withheld is denied whole and
/// one they consented to is served even when the caller withheld it. The
/// consents are also recorded for the output boundary of the call `fut` runs
/// in ([`subject_consents`]), so the backstop filtering the call's result
/// applies the consents its reads ran under. A call that reads several people
/// this way records every provider any of them withheld.
pub async fn with_subject_consent<F: Future>(withheld: BTreeSet<String>, fut: F) -> F::Output {
    let withheld = lowercased(withheld);
    record_subject(&withheld);
    AI_CONSENT_WITHHELD.scope(Arc::new(withheld), fut).await
}

/// Run `fut`, returning what the people it read other than its caller
/// withheld from AI ([`with_subject_consent`]): `None` when it read only its
/// caller's data.
///
/// The tool executor wraps every tool body in this, and filters the body's
/// result under what it returns. What a nested call records reaches the call
/// enclosing it too: the outer result carries what the inner one returned.
pub async fn subject_consents<F: Future>(fut: F) -> (F::Output, Option<BTreeSet<String>>) {
    let slot = Arc::new(Mutex::new(None));
    let output = SUBJECT_CONSENTS.scope(Arc::clone(&slot), fut).await;
    let recorded = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(withheld) = &recorded {
        record_subject(withheld);
    }
    (output, recorded)
}

/// Add `withheld` to what the enclosing call recorded of the people it read.
/// Nothing happens outside such a call.
fn record_subject(withheld: &BTreeSet<String>) {
    let _ = SUBJECT_CONSENTS.try_with(|slot| {
        slot.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(BTreeSet::new)
            .extend(withheld.iter().cloned());
    });
}

/// The providers whose data the athlete has not consented to hand to AI, and
/// the consent classes ([`SENSITIVE_T2`]) they have not given.
///
/// As the current call declared them ([`with_ai_consent`]), or every key
/// whose notice is a consent to AI use when it declared none. A reader that
/// matches it against provider names reads [`withheld_providers`].
#[must_use]
pub fn ai_consent_withheld() -> Arc<BTreeSet<String>> {
    AI_CONSENT_WITHHELD
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Arc::new(ai_consent_backends().map(str::to_ascii_lowercase).collect()))
}

/// The providers alone of [`ai_consent_withheld`].
///
/// The set every reader that matches it against provider names reads, so a
/// consent class ([`SENSITIVE_T2`]) never reads as a withheld provider: it is
/// no provider, and only the tier gate ([`tier_readable`]) answers it.
#[must_use]
pub fn withheld_providers() -> BTreeSet<String> {
    ai_consent_withheld()
        .iter()
        .filter(|key| !is_consent_class(key))
        .cloned()
        .collect()
}

/// Refuse a model's or a human coach's untagged read of a provider lacking the
/// athlete's AI consent.
///
/// For the reads that carry no item provenance (profile, calendar),
/// when the athlete has not consented to AI use of `provider`'s data. The
/// refusal is tallied like a dropped item, so the result carries the neutral
/// withheld note rather than a bare failure.
///
/// # Errors
///
/// [`ErrorCode::ResourceNotFound`](pierre_core::errors::ErrorCode::ResourceNotFound)
/// when a model or a human coach reads and `provider`'s AI consent is withheld.
pub fn consented_read(provider: &str) -> AppResult<()> {
    let for_model = exposure().is_some_and(|gate| gate.to_model);
    if !for_model || !withheld_providers().contains(&provider.to_ascii_lowercase()) {
        return Ok(());
    }
    let mut withheld = Withheld {
        dropped: 1,
        ..Withheld::default()
    };
    withheld.sources.insert(provider.to_ascii_lowercase());
    record_withheld(withheld);
    Err(AppError::not_found(format!(
        "{provider} data is withheld from AI: the athlete has not consented to its AI use"
    )))
}

/// Why a read of a sensitive record is refused here, or `None` when it may
/// be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TierRefusal {
    /// Served over an external transport: a delegated grant's ceiling is T1.
    External,
    /// A model's or a human coach's read the athlete has not allowed.
    Withheld,
}

/// Why a `tier` record read now is refused, or `None` when it may be read.
///
/// T0 and T1 are always read (D1). A T2 or T3 record is refused over an
/// external transport (MCP, A2A, an API key), consent or not. A T2 record is
/// refused under a model read or a human coach's read
/// ([`for_human_coach`](super::for_human_coach)) while the athlete withholds
/// [`SENSITIVE_T2`] ([`ai_consent_withheld`]); a T3 record has no consent
/// class, so every such read refuses it. Anywhere else — the athlete's own
/// screens — every tier is read.
fn tier_refusal(tier: Tier) -> Option<TierRefusal> {
    let gate = exposure()?;
    let class = match tier {
        Tier::T0 | Tier::T1 => return None,
        Tier::T2 => Some(SENSITIVE_T2),
        Tier::T3 => None,
    };
    if gate.external {
        return Some(TierRefusal::External);
    }
    let withheld = gate.to_model && class.is_none_or(|class| ai_consent_withheld().contains(class));
    withheld.then_some(TierRefusal::Withheld)
}

/// Tally one sensitive record or field `refusal` withheld.
fn tally_refusal(refusal: TierRefusal) {
    record_withheld(match refusal {
        TierRefusal::External => Withheld {
            off_interface: 1,
            ..Withheld::default()
        },
        TierRefusal::Withheld => Withheld {
            sensitive_reduced: 1,
            ..Withheld::default()
        },
    });
}

/// Whether a `tier` record read now may be served, the gate every sensitive
/// read asks, for a filter that nulls a field: nothing is tallied,
/// so the caller tallies each row it nulls ([`withhold_tier`]).
#[must_use]
pub fn tier_readable(tier: Tier) -> bool {
    tier_refusal(tier).is_none()
}

/// Tally one field of `tier` a filter nulled because [`tier_readable`] said
/// no: under the athlete's tier consent as a sensitive reduction, over an
/// external transport as data not served over this interface.
pub fn withhold_tier(tier: Tier) {
    if let Some(refusal) = tier_refusal(tier) {
        tally_refusal(refusal);
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_core::transport::{Transport, TransportPolicy};

    use super::*;

    use crate::ai_scope::{ai_read, filter_activities, serve_over, tallied};
    use crate::provider_terms::WHOOP;

    /// A sensitive read as a refusing reader asks it: [`tier_readable`],
    /// tallied once when refused, and answered as the read refusal a
    /// tool returns.
    fn consented_tier(tier: Tier) -> AppResult<()> {
        let Some(refusal) = tier_refusal(tier) else {
            return Ok(());
        };
        tally_refusal(refusal);
        Err(AppError::not_found(match refusal {
            TierRefusal::External => "Sensitive health records are not served over this interface",
            TierRefusal::Withheld => {
                "Sensitive health records are withheld from AI: the athlete has not allowed their AI use"
            }
        }))
    }

    /// WHOOP's declared terms, and no others.
    struct WhoopTerms;

    impl ProviderTerms for WhoopTerms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == "whoop").then_some(&WHOOP)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == "whoop").then_some(TransportPolicy::AnyTransport)
        }
    }

    fn workout(id: &str, provider: &str) -> Activity {
        let start = Utc.with_ymd_and_hms(2026, 9, 30, 6, 0, 0).unwrap();
        ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3600, provider).build()
    }

    fn week() -> Vec<Activity> {
        vec![workout("w", "whoop"), workout("g", "garmin")]
    }

    fn ids(activities: &[Activity]) -> Vec<&str> {
        activities.iter().map(Activity::id).collect()
    }

    #[tokio::test]
    async fn a_withheld_ai_consent_denies_the_provider_and_the_athlete_still_sees_it() {
        let withdrawn = BTreeSet::from(["WHOOP".to_owned()]);
        let (kept, withheld) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                withdrawn,
                ai_read(async { filter_activities(&WhoopTerms, week()) }),
            ),
        )
        .await;
        assert_eq!(ids(&kept), vec!["g"], "no WHOOP workout reaches the model");
        assert!(withheld.sources.contains("whoop"), "{:?}", withheld.sources);

        let consented = serve_over(
            Transport::WebApp,
            with_ai_consent(
                BTreeSet::new(),
                ai_read(async { filter_activities(&WhoopTerms, week()) }),
            ),
        )
        .await
        .0;
        assert_eq!(ids(&consented), vec!["w", "g"]);

        let route = filter_activities(&WhoopTerms, week());
        assert_eq!(
            route.len(),
            2,
            "the athlete's own screens keep every workout"
        );
    }

    #[tokio::test]
    async fn an_ai_read_that_declared_no_consent_withholds_every_ai_consent_provider() {
        assert!(ai_consent_backends().any(|backend| backend == "whoop"));
        let (kept, _) = serve_over(
            Transport::WebApp,
            ai_read(async { filter_activities(&WhoopTerms, week()) }),
        )
        .await;
        assert_eq!(
            ids(&kept),
            vec!["g"],
            "a forgotten declaration withholds, never hands over"
        );
    }

    #[tokio::test]
    async fn an_untagged_model_read_of_a_provider_without_consent_is_refused_and_tallied() {
        let withheld = || BTreeSet::from(["whoop".to_owned()]);
        let (refused, tally) = serve_over(
            Transport::WebApp,
            with_ai_consent(withheld(), ai_read(async { consented_read("whoop") })),
        )
        .await;
        assert_eq!(
            refused.expect_err("bug: no profile for a model").code,
            ErrorCode::ResourceNotFound
        );
        assert_eq!(tally.dropped, 1);

        let (athlete, _) = tallied(with_ai_consent(withheld(), async {
            consented_read("whoop")
        }))
        .await;
        assert!(athlete.is_ok(), "outside a model read the athlete reads it");
    }

    fn t2_withheld() -> BTreeSet<String> {
        BTreeSet::from([SENSITIVE_T2.to_owned()])
    }

    #[tokio::test]
    async fn a_model_reads_t2_only_once_its_class_is_given_and_t3_never() {
        let (refused, tally) = serve_over(
            Transport::WebApp,
            with_ai_consent(t2_withheld(), ai_read(async { consented_tier(Tier::T2) })),
        )
        .await;
        assert_eq!(
            refused.expect_err("bug: T2 withheld from a model").code,
            ErrorCode::ResourceNotFound
        );
        assert_eq!(
            (tally.sensitive_reduced, tally.dropped, tally.off_interface),
            (1, 0, 0),
            "the refusal is tallied under its own reason, never the provider terms: {tally:?}"
        );

        let (given, tally) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                BTreeSet::new(),
                ai_read(async { (consented_tier(Tier::T2), tier_readable(Tier::T3)) }),
            ),
        )
        .await;
        assert!(given.0.is_ok(), "T2 is given: the model reads it");
        assert!(!given.1, "bug: T3 has no consent yet, so no model reads it");
        assert!(tally.is_empty(), "the predicate tallies nothing: {tally:?}");
    }

    #[tokio::test]
    async fn t0_and_t1_are_read_on_every_gate_without_consent() {
        for transport in [Transport::WebApp, Transport::McpHttp, Transport::ApiKey] {
            let (read, tally) = serve_over(
                transport,
                with_ai_consent(
                    t2_withheld(),
                    ai_read(async { (consented_tier(Tier::T0), consented_tier(Tier::T1)) }),
                ),
            )
            .await;
            assert!(read.0.is_ok() && read.1.is_ok(), "{transport:?}");
            assert!(tally.is_empty(), "{transport:?}: {tally:?}");
        }
    }

    #[tokio::test]
    async fn an_external_transport_never_reads_t2_even_with_consent() {
        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            let (read, tally) = serve_over(
                transport,
                with_ai_consent(
                    BTreeSet::new(),
                    ai_read(async {
                        withhold_tier(Tier::T2);
                        consented_tier(Tier::T2)
                    }),
                ),
            )
            .await;
            assert!(
                read.is_err(),
                "bug: a delegated grant's ceiling is T1 ({transport:?})"
            );
            assert_eq!(
                (tally.off_interface, tally.sensitive_reduced),
                (2, 0),
                "{transport:?}: {tally:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_human_coachs_read_needs_the_athletes_consent_and_the_athletes_own_screen_does_not() {
        let (coach, tally) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                t2_withheld(),
                tallied(super::super::for_human_coach(async {
                    consented_tier(Tier::T2)
                })),
            ),
        )
        .await;
        assert!(
            coach.is_err(),
            "bug: a human coach reads T2 only under consent"
        );
        assert_eq!(
            tally.sensitive_reduced, 1,
            "bug: the coach's refusal is tallied once: {tally:?}"
        );

        let (consented, _) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                BTreeSet::new(),
                tallied(super::super::for_human_coach(async {
                    consented_tier(Tier::T2)
                })),
            ),
        )
        .await;
        assert!(
            consented.is_ok(),
            "the athlete gave T2: their coach reads it"
        );

        let (own, tally) = serve_over(
            Transport::WebApp,
            with_ai_consent(
                t2_withheld(),
                tallied(async { (consented_tier(Tier::T2), consented_tier(Tier::T3)) }),
            ),
        )
        .await;
        assert!(
            own.0.is_ok() && own.1.is_ok(),
            "the athlete's own screens read every tier"
        );
        assert!(
            tally.is_empty(),
            "bug: the athlete's own read withholds nothing: {tally:?}"
        );
    }

    #[tokio::test]
    async fn the_provider_view_of_the_consent_set_never_names_a_class() {
        let (providers, full) = with_ai_consent(
            BTreeSet::from(["whoop".to_owned(), SENSITIVE_T2.to_owned()]),
            async { (withheld_providers(), ai_consent_withheld()) },
        )
        .await;
        assert_eq!(
            providers,
            BTreeSet::from(["whoop".to_owned()]),
            "bug: the provider view names providers alone"
        );
        assert!(full.contains(SENSITIVE_T2), "the tier gate still reads it");
    }

    #[tokio::test]
    async fn an_ai_read_that_declared_no_consent_withholds_t2() {
        let (read, _) = serve_over(
            Transport::WebApp,
            ai_read(async { consented_tier(Tier::T2) }),
        )
        .await;
        assert!(read.is_err(), "a forgotten declaration withholds the class");
    }

    /// A model read of `week()` made for a caller whose consents are
    /// `caller`, of a subject whose consents are `subject`: what the read
    /// kept, and what the call's output boundary records.
    async fn read_as_subject(
        caller: BTreeSet<String>,
        subject: BTreeSet<String>,
    ) -> (Vec<Activity>, Option<BTreeSet<String>>) {
        let ((kept, _), boundary) = serve_over(
            Transport::WebApp,
            subject_consents(with_ai_consent(
                caller,
                ai_read(with_subject_consent(subject, async {
                    filter_activities(&WhoopTerms, week())
                })),
            )),
        )
        .await;
        (kept, boundary)
    }

    #[tokio::test]
    async fn a_subjects_withheld_consent_denies_the_provider_whatever_the_caller_gave() {
        let (kept, boundary) =
            read_as_subject(BTreeSet::new(), BTreeSet::from(["WHOOP".to_owned()])).await;
        assert_eq!(
            ids(&kept),
            vec!["g"],
            "the subject withheld WHOOP: no WHOOP workout reaches the caller's model"
        );
        assert_eq!(
            boundary,
            Some(BTreeSet::from(["whoop".to_owned()])),
            "the call's output boundary applies the subject's consents"
        );
    }

    #[tokio::test]
    async fn a_subjects_consent_serves_the_provider_the_caller_withheld() {
        let (kept, boundary) =
            read_as_subject(BTreeSet::from(["whoop".to_owned()]), BTreeSet::new()).await;
        assert_eq!(
            ids(&kept),
            vec!["w", "g"],
            "the subject consented to WHOOP: the caller's own withdrawal hides nothing of theirs"
        );
        assert_eq!(
            boundary,
            Some(BTreeSet::new()),
            "the boundary withholds nothing the subject consented to"
        );
    }

    #[tokio::test]
    async fn a_call_that_reads_only_its_caller_records_no_subject() {
        let ((), boundary) = subject_consents(with_ai_consent(BTreeSet::new(), async {})).await;
        assert_eq!(
            boundary, None,
            "the caller's own consents govern its own call"
        );
    }

    #[tokio::test]
    async fn every_subject_a_call_read_and_its_nested_calls_read_reaches_its_boundary() {
        let ((), outer) = subject_consents(async {
            with_subject_consent(BTreeSet::from(["whoop".to_owned()]), async {}).await;
            let ((), inner) = subject_consents(with_subject_consent(
                BTreeSet::from(["oura".to_owned()]),
                async {},
            ))
            .await;
            assert_eq!(
                inner,
                Some(BTreeSet::from(["oura".to_owned()])),
                "the nested call's boundary holds what it read alone"
            );
        })
        .await;
        assert_eq!(
            outer,
            Some(BTreeSet::from(["oura".to_owned(), "whoop".to_owned()])),
            "the outer call's result carries both reads, so it withholds what either withheld"
        );
    }
}
