// ABOUTME: Offline guards for the live incident lane — fixture, ground truth, corpus order and the reproduce rule
// ABOUTME: The lane itself is the live-e2e target live_incident_corpus_test; these run in every default pass

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Offline guards for the live-model incident corpus.
//!
//! The corpus — every 2026-08 coaching incident replayed against a REAL model
//! over production's transport, graded on the DELIVERED body — is
//! `tests/live/live_incident_corpus_test.rs`. It needs a real model, so it is a
//! `live-e2e` target that a default build never compiles and that fails,
//! never skips, when its model is absent (carnet#805). Its module docs say why
//! it exists and how to run it.
//!
//! What stays here is what can be checked without a model and must be, because
//! no live run would ever surface it: a ground truth that disagrees with the
//! seeded fixture makes the judge confidently wrong, a corpus reordered so the
//! introduction episode no longer opens the DM reds a correct pipeline, a
//! fixture whose primary provider holds no token turns every episode into the
//! reconnect sentence, and the rule deciding which findings red the lane is the
//! one part of it that decides whether anyone trusts it. The corpus, fixture
//! and classifier these guard live in `tests/helpers/live_incident_corpus.rs`,
//! shared with the live target.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;
#[cfg(feature = "client-messaging")]
#[path = "helpers/live_incident_corpus.rs"]
mod live_incident_corpus;

#[cfg(feature = "client-messaging")]
mod live_incident_eval {
    use crate::common::create_test_server_resources;
    use crate::live_incident_corpus::{
        athlete_run_metres, classify_findings, ground_truth, km_range, peer_run_metres,
        seed_fixture, Finding, ATHLETE_RUN_DAYS, CORPUS, PEER_RUN_DAYS, PEER_RUN_METRES,
        PEER_RUN_PACE, PEER_RUN_SECONDS,
    };
    use pierre_providers::activity_source::resolve_activity_source;
    use pierre_providers::registry::global_registry;
    use serial_test::serial;

    /// The ground truth's stated pace must be what the seeded run arithmetically
    /// is.
    ///
    /// This runs in ordinary CI, unlike the corpus itself, because it is the one
    /// thing that can silently corrupt every judged verdict: a ground truth that
    /// disagrees with the fixture makes the judge confidently wrong rather than
    /// merely uninformed, and no amount of live running would surface it — the
    /// judge would just keep marking correct replies as fabrications, exactly as
    /// it did before it was given any ground truth at all.
    #[test]
    fn the_stated_peer_pace_matches_the_seeded_run() {
        let minutes = PEER_RUN_SECONDS as f64 / 60.0;
        let km = PEER_RUN_METRES / 1_000.0;
        let pace = minutes / km;
        let mins = pace.trunc() as u64;
        let secs = ((pace - pace.trunc()) * 60.0).round() as u64;
        let derived = format!("{mins}min{secs:02}/km");
        assert_eq!(
            derived, PEER_RUN_PACE,
            "PEER_RUN_PACE says {PEER_RUN_PACE} but {PEER_RUN_SECONDS}s over \
             {PEER_RUN_METRES}m is {derived} — the judge would be told a figure the \
             fixture cannot produce"
        );
    }

    /// The ground truth's stated distance ranges must be the ones the fixture
    /// seeds — endpoints a day actually produces, not a formula's intercept.
    ///
    /// The same class as the pace guard and for the same reason: a range stated
    /// wider than the data widens the band the judge accepts, so a figure the
    /// agent invented lands inside it and is graded honest. The peer's runs are
    /// `day * 1.5 km + 7 km` over days 2 and 4 — 10 km and 13 km. "7 km" is the
    /// intercept, a value no seeded day carries.
    #[test]
    fn the_stated_run_distances_match_the_seeded_activities() {
        let athlete = km_range(&ATHLETE_RUN_DAYS, athlete_run_metres);
        let peer = km_range(&PEER_RUN_DAYS, peer_run_metres);
        assert_eq!(
            athlete, "12-20",
            "the athlete's seeded runs span {athlete} km, not 12-20"
        );
        assert_eq!(
            peer, "10-13",
            "the peer's seeded runs span {peer} km, not 10-13"
        );

        let evidence = ground_truth();
        assert!(
            evidence.contains(&format!("runs of {athlete} km on")),
            "the ground truth does not state the athlete's seeded {athlete} km range:\n{evidence}"
        );
        assert!(
            evidence.contains(&format!("runs of {peer} km")),
            "the ground truth does not state the peer's seeded {peer} km range:\n{evidence}"
        );
    }

    /// The introduction episode must be the first one posted to the DM.
    ///
    /// Every direct-message episode in a pass shares one conversation, and the
    /// agent introduces itself only until a reply there has named it. An
    /// episode moved above this one would take the introduction, and this
    /// episode's gating `AnyOf(["eval coach"])` would then red the nightly lane
    /// on a correct pipeline.
    #[test]
    fn the_introduction_episode_opens_the_direct_message() {
        let first_dm = CORPUS
            .iter()
            .find(|episode| !episode.group)
            .expect("the corpus has direct-message episodes");
        assert_eq!(
            first_dm.name, "first_reply_introduction",
            "the first DM episode is {:?}; only the first one meets an unintroduced agent",
            first_dm.name
        );
    }

    /// The provider `resolve_activity_source` elects for the seeded athlete must be
    /// one the fixture holds a token for.
    ///
    /// Runs in ordinary CI, like the pace guard, because a broken election is
    /// invisible in a delivered reply: a token-less primary makes every live
    /// fetch signal re-auth, the tool loop short-circuits, and each episode is
    /// answered by one deterministic reconnect sentence that reads as fluent
    /// coaching prose. Three unrelated episodes then fail on byte-identical
    /// copy, and the corpus reports it as three model regressions. Election
    /// order is a property of the seeding, so it is asserted rather than
    /// watched for.
    #[tokio::test]
    #[serial]
    async fn the_fixtures_primary_provider_holds_a_token() {
        let resources = create_test_server_resources().await.unwrap();
        let fixture = seed_fixture(&resources).await;

        let primary = resolve_activity_source(
            resources.common.repos.provider_connections.as_ref(),
            &global_registry(),
            fixture.athlete,
            Some(fixture.athlete_tenant),
        )
        .await
        .unwrap()
        .expect("the seeded athlete must have provider connections");

        let tokens = resources
            .common
            .repos
            .oauth_tokens
            .get_tokens(fixture.athlete, Some(fixture.athlete_tenant))
            .await
            .unwrap();
        let with_tokens: Vec<&str> = tokens.iter().map(|t| t.provider.as_str()).collect();

        assert_eq!(
            with_tokens,
            vec!["sciotte"],
            "the fixture seeds exactly one provider token; seeding another is fine, but the \
             connection registered LAST in `seed_fixture` still has to be one of them"
        );
        assert!(
            with_tokens.contains(&primary.provider.as_str()),
            "the fixture elects {:?} as the athlete's primary provider but holds tokens only \
             for {with_tokens:?} — every live fetch resolves to re-auth and the corpus grades \
             the reconnect sentence instead of the coach",
            primary.provider
        );
    }

    /// Build a finding for the classifier tests.
    fn finding(
        episode: &'static str,
        turn_index: usize,
        kind: &'static str,
        gates: bool,
    ) -> Finding {
        Finding {
            episode,
            incident: "test",
            turn_index,
            user: "test",
            kind,
            gates,
            detail: format!("{kind} fired"),
            delivered: "delivered".to_owned(),
        }
    }

    /// The 2026-08-26 evidence, replayed through the rule: the missing chart
    /// block failed all four runs and must red; the empty reply moved between
    /// episodes run to run and must not.
    #[test]
    fn a_finding_reds_the_lane_only_when_it_reproduces() {
        let findings = vec![
            finding("group_chart_ask", 1, "chart_missing", true),
            finding("group_chart_ask", 1, "chart_missing", true),
            finding("weekly_summary", 1, "banned_phrase", true),
        ];
        let out = classify_findings(&findings, 3);

        assert_eq!(
            out.reproduced.len(),
            1,
            "expected exactly the reproduced finding to gate, got {:?}",
            out.reproduced
                .iter()
                .map(|(_, f)| f.kind)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            out.reproduced[0].0, 2,
            "should report how many passes saw it"
        );
        assert_eq!(out.reproduced[0].1.episode, "group_chart_ask");
        assert_eq!(
            out.flaky.len(),
            1,
            "the one-off must be reported, not gating"
        );
        assert_eq!(out.flaky[0].1.episode, "weekly_summary");
    }

    /// A defect that fails EVERY pass is the strongest possible signal and must
    /// never be filtered out by the very rule meant to remove noise.
    #[test]
    fn a_finding_in_every_pass_always_reds() {
        let findings: Vec<Finding> = (0..3)
            .map(|_| finding("chart_with_invented_accent", 0, "chart_missing", true))
            .collect();
        let out = classify_findings(&findings, 3);
        assert_eq!(out.reproduced.len(), 1);
        assert_eq!(out.reproduced[0].0, 3);
        assert!(out.flaky.is_empty());
    }

    /// The judge and the length check report but never gate — an LLM grading an
    /// LLM samples twice, and reply length is style, not correctness.
    #[test]
    fn ungated_assertions_never_red_even_when_unanimous() {
        let findings: Vec<Finding> = (0..3)
            .flat_map(|_| {
                [
                    finding("capability_claim", 0, "judge", false),
                    finding("weekly_summary", 1, "too_short", false),
                ]
            })
            .collect();
        let out = classify_findings(&findings, 3);
        assert!(
            out.reproduced.is_empty(),
            "a non-gating assertion reached the gate: {:?}",
            out.reproduced
                .iter()
                .map(|(_, f)| f.kind)
                .collect::<Vec<_>>()
        );
        assert_eq!(out.ungated.len(), 2, "both must still be reported");
        assert!(out.ungated.iter().all(|(seen, _)| *seen == 3));
    }

    /// Findings are keyed by identity, never by rendered text: the detail
    /// carries per-run numbers ("79 chars") and judge prose that is reworded on
    /// every call, so text-keyed grouping would never reach the threshold.
    #[test]
    fn differing_detail_text_still_groups_as_one_finding() {
        let mut a = finding("weekly_summary", 1, "too_short", true);
        let mut b = finding("weekly_summary", 1, "too_short", true);
        a.detail = "delivered body is 79 chars, expected at least 120".to_owned();
        b.detail = "delivered body is 83 chars, expected at least 120".to_owned();
        let findings = [a, b];
        let out = classify_findings(&findings, 3);
        assert_eq!(
            out.reproduced.len(),
            1,
            "same defect must group despite differing text"
        );
        assert_eq!(out.reproduced[0].0, 2);
    }

    /// Same assertion failing on two different turns is two defects, not one
    /// reproduced defect — collapsing them would red the lane on a pair of
    /// unrelated single-pass draws.
    #[test]
    fn the_same_assertion_on_different_turns_does_not_reproduce() {
        let findings = vec![
            finding("group_chart_ask", 0, "banned_phrase", true),
            finding("two_provider_day", 0, "banned_phrase", true),
        ];
        let out = classify_findings(&findings, 3);
        assert!(
            out.reproduced.is_empty(),
            "two different turns were collapsed into one reproduced finding"
        );
        assert_eq!(out.flaky.len(), 2);
    }

    /// A single-pass local run keeps the old zero-tolerance behaviour, so
    /// `LIVE_INCIDENT_EVAL_ATTEMPTS=1` stays useful for reproducing one defect.
    #[test]
    fn a_single_pass_run_gates_on_one_occurrence() {
        let findings = vec![finding("group_chart_ask", 1, "chart_missing", true)];
        let out = classify_findings(&findings, 1);
        assert_eq!(out.reproduced.len(), 1);
        assert!(out.flaky.is_empty());
    }
}
