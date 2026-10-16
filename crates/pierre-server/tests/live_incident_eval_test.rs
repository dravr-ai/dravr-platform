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
//! reconnect sentence, a per-turn wait shorter than the server's turn ceiling
//! grades a late reply as the next turn's answer, and the rule deciding which
//! findings red the lane is the one part of it that decides whether anyone
//! trusts it. The corpus, fixture, wait and classifier these guard live in
//! `tests/helpers/live_incident_corpus.rs`, shared with the live target.

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
        athlete_runs, classify_findings, connection_state, ground_truth, peer_runs,
        reconnect_alert, seed_fixture, sunday_ride_start, turn_timeout, Expect, Finding,
        ATHLETE_DISPLAY, ATHLETE_LIVE_CONNECTION, CORPUS, PEER_DISPLAY, PEER_RUN_METRES,
        PEER_RUN_PACE, PEER_RUN_SECONDS, PHIL_ALIAS_DISPLAY,
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use dravr_canot::channels::slack::transport::SlackTransport;
    use dravr_canot::transport::TransportAdapter;
    use http::HeaderMap;
    use pierre_chat_pipeline::stages::viz_blocks::granted_visuals;
    use pierre_mcp_server::services::messaging_ingress::turn_watchdog;
    use pierre_providers::activity_source::resolve_activity_source;
    use pierre_providers::core::ActivityQueryParams;
    use pierre_providers::registry::global_registry;
    use pierre_services::agent_selection::AgentSelectionSource;
    use pierre_services::conversation_forge::{
        forge_conversation, selected_or_system_agent, ForgeAgent, ForgeParams, SlotQuota,
    };
    use pierre_tool_runtime::activity_fetch::fetch_provider_activities;
    use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serial_test::serial;
    use std::collections::BTreeSet;
    use std::env;
    use std::sync::Arc;

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

    /// The ground truth must date every run the fixture seeds, with the
    /// distance it carries.
    ///
    /// The same class as the pace guard and for the same reason: evidence that
    /// is vaguer than the data makes the judge wrong in one direction or the
    /// other. Shown ranges with no dates, it graded the agent's correct
    /// «12 km en 1h10 hier» for JF as invented, because nothing it was shown
    /// put a run of JF's on yesterday — a finding about the evidence, filed
    /// against the model.
    #[test]
    fn the_ground_truth_dates_every_seeded_run() {
        let evidence = ground_truth();
        for (who, runs) in [
            ("Athlete JF", athlete_runs()),
            ("Peer Philippe Tremblay", peer_runs()),
        ] {
            for run in runs {
                let date = run.start.format("%Y-%m-%d, started %H:%M UTC").to_string();
                let km = run.metres / 1_000.0;
                assert!(
                    evidence.lines().any(|line| line.starts_with(who)
                        && line.contains(&date)
                        && line.contains(&format!(" {km} km in "))),
                    "the ground truth has no line for {who}'s {km} km run on {date} \
                     ({}):\n{evidence}",
                    run.id
                );
            }
        }
    }

    /// No two seeded activities start at the same time of day.
    ///
    /// When every start was the fixture clock minus whole days, the whole
    /// history began at the minute the lane ran, and a challenged agent
    /// retracting its correct «14 h 54» was graded against a ledger whose
    /// one start time was every row's (carnet#1038).
    #[test]
    fn every_seeded_activity_starts_at_its_own_time_of_day() {
        let mut starts = BTreeSet::from([sunday_ride_start().format("%H:%M").to_string()]);
        for run in athlete_runs().into_iter().chain(peer_runs()) {
            let time = run.start.format("%H:%M").to_string();
            assert!(
                starts.insert(time.clone()),
                "{} starts at {time} UTC, the time of day another seeded activity starts",
                run.id
            );
        }
    }

    /// Every activity the athlete is served starts when the judge's ledger
    /// says it does.
    ///
    /// Read back from the activity cache, the store the agent's tools serve,
    /// so this holds only while the seed and the ledger measure from one
    /// clock. When each read `Utc::now()` on its own, the ledger could carry
    /// no start at all, and the agent quoting a run's real 09:52 start was
    /// graded as inventing it.
    #[tokio::test]
    #[serial]
    async fn every_cached_activity_starts_where_the_ledger_says() {
        let resources = create_test_server_resources().await.unwrap();
        let fixture = seed_fixture(&resources).await;
        let evidence = ground_truth();
        let cached = resources
            .common
            .repos
            .activity_cache
            .get_cached_activities(
                fixture.athlete,
                &fixture.athlete_tenant,
                None,
                Utc::now() - ChronoDuration::days(60),
                Utc::now(),
                500,
            )
            .await
            .unwrap();
        assert!(
            cached.len() > athlete_runs().len(),
            "expected every seeded run plus the Sunday twins, got {} rows",
            cached.len()
        );
        for activity in &cached {
            let start = activity
                .start_date()
                .format("%Y-%m-%d, started %H:%M UTC")
                .to_string();
            assert!(
                evidence
                    .lines()
                    .any(|line| line.starts_with("Athlete JF") && line.contains(&start)),
                "{} is served as starting {start}, which no ledger line states:\n{evidence}",
                activity.id()
            );
        }
    }

    /// The facts the judge got wrong are stated in its evidence: JF's run
    /// yesterday (12 km in 1h10, on every day but a Monday, whose yesterday
    /// is the Sunday the fixture keeps for the ride twin), and Philippe's run
    /// yesterday, 6.1 km at [`PEER_RUN_PACE`].
    #[test]
    fn the_ground_truth_states_yesterdays_runs() {
        let evidence = ground_truth();
        let yesterday: Vec<&str> = evidence
            .lines()
            .filter(|line| line.contains(", yesterday, "))
            .collect();
        let jf_yesterday = athlete_runs().into_iter().find(|run| run.days_ago == 1);
        if let Some(run) = jf_yesterday {
            assert_eq!((run.metres, run.seconds), (12_000.0, 4_200));
            assert!(
                yesterday
                    .iter()
                    .any(|line| line.starts_with("Athlete JF") && line.contains(" 12 km in 1h10 ")),
                "JF's seeded run yesterday (12 km, 4200 s) is not in the evidence: {yesterday:?}"
            );
        }
        assert!(
            yesterday
                .iter()
                .any(|line| line.starts_with("Peer Philippe Tremblay")
                    && line.contains(" 6.1 km in 0h53 ")
                    && line.contains(PEER_RUN_PACE)),
            "Philippe's seeded run yesterday (6.1 km, {PEER_RUN_SECONDS} s, {PEER_RUN_PACE}) \
             is not in the evidence: {yesterday:?}"
        );
    }

    /// The introduction episode must be the first one posted to the DM.
    ///
    /// Every direct-message episode in a pass shares one conversation, and the
    /// agent introduces itself only until a reply there has named it. An
    /// episode moved above this one would take the introduction, and this
    /// episode's gating `NamesAgent(EVAL_COACH_TITLE)` would then red the nightly lane
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
            vec![ATHLETE_LIVE_CONNECTION],
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

    /// The ledger states every roster member's connections as the fixture
    /// seeds them, and quotes the reconnect alert the agent is shown for the
    /// peer word for word.
    ///
    /// Read back from the store, not from the constants the ledger is built
    /// from, so a seed that registers or credentials a connection the ledger
    /// does not name fails here. The alert half drives the peer fetch
    /// `get_group_member_activities` makes — every one of his connections,
    /// through `fetch_provider_activities` — and reads the group snapshot
    /// after it: a tokenless connection is flagged on that fetch, and the
    /// snapshot's `needs_reauth_providers` is what the group context renders
    /// under Connection alerts. In the 2026-10-15 run the agent said
    /// Philippe's Strava needed reconnecting, which was exactly that alert,
    /// and the judge, told only about JF's connections, graded it invented
    /// (carnet#1037).
    #[tokio::test]
    #[serial]
    async fn the_ground_truth_states_every_members_connections_as_seeded() {
        let resources = create_test_server_resources().await.unwrap();
        let fixture = seed_fixture(&resources).await;
        let evidence = ground_truth();
        let repos = &resources.common.repos;

        for (display, user) in [
            (ATHLETE_DISPLAY, fixture.athlete),
            (PEER_DISPLAY, fixture.peer),
            (PHIL_ALIAS_DISPLAY, fixture.phil_alias),
        ] {
            let connections = repos
                .provider_connections
                .get_for_user(user, None)
                .await
                .unwrap();
            let tokens: Vec<String> = repos
                .oauth_tokens
                .get_tokens(user, None)
                .await
                .unwrap()
                .into_iter()
                .map(|token| token.provider)
                .collect();
            let seeded: BTreeSet<String> = connections
                .iter()
                .map(|c| connection_state(&c.provider, tokens.contains(&c.provider)))
                .collect();
            assert!(!seeded.is_empty(), "{display} has no seeded connection");

            let prefix = format!("Connections of {display}: ");
            let line = evidence
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .unwrap_or_else(|| {
                    panic!("the ledger states no connections for {display}:\n{evidence}")
                });
            let stated: BTreeSet<String> = line
                .trim_end_matches('.')
                .split("; ")
                .map(str::to_owned)
                .collect();
            assert_eq!(
                stated, seeded,
                "the ledger states {display}'s connections as {stated:?} but the fixture \
                 seeds {seeded:?}"
            );
        }

        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let before =
            fetch_member_snapshots(&runtime, &[fixture.peer], fixture.athlete_tenant).await;
        assert!(
            before[0].needs_reauth_providers.is_empty(),
            "the peer's connections read as needing reconnection before anything fetched \
             them: {:?}",
            before[0].needs_reauth_providers
        );
        let params = ActivityQueryParams {
            limit: Some(10),
            offset: None,
            before: None,
            after: None,
        };
        for conn in repos
            .provider_connections
            .get_for_user(fixture.peer, None)
            .await
            .unwrap()
        {
            fetch_provider_activities(
                &runtime,
                &conn.provider,
                fixture.peer,
                &conn.tenant_id,
                &params,
            )
            .await;
        }
        let after = fetch_member_snapshots(&runtime, &[fixture.peer], fixture.athlete_tenant).await;
        let flagged: Vec<&str> = after[0]
            .needs_reauth_providers
            .iter()
            .map(String::as_str)
            .collect();
        assert!(
            !flagged.is_empty(),
            "fetching the peer flagged none of his tokenless connections; the ledger tells \
             the judge the agent is shown a reconnect alert for him"
        );
        // The alert as the production group context renders it for the
        // athlete, not as `reconnect_alert` restates it: a reword on either
        // side fails here rather than leaving the ledger quoting a line the
        // agent is never shown.
        let context = resources
            .common
            .group_service
            .inject_group_context(
                "",
                &fixture.group_agent.to_string(),
                fixture.athlete,
                fixture.athlete_tenant,
                Some(&fixture.group.to_string()),
                &after,
            )
            .await
            .unwrap();
        let alerts = context
            .split_once("## Connection alerts\n")
            .map_or("", |(_, section)| {
                section.split("\n\n##").next().unwrap_or(section)
            });
        let rendered = alerts
            .lines()
            .filter_map(|line| line.strip_prefix("- "))
            .find(|line| line.starts_with(PEER_DISPLAY))
            .unwrap_or_else(|| {
                panic!("the group context shows no Connection alert for {PEER_DISPLAY}:\n{context}")
            });
        assert_eq!(
            rendered,
            reconnect_alert(&after[0].display_name, &flagged),
            "the ledger's alert format has drifted from the group context's"
        );
        let alert = format!("«{rendered}»");
        assert!(
            evidence.contains(&alert),
            "once the peer is fetched the agent is shown {alert}, which the ledger does not \
             quote:\n{evidence}"
        );
    }

    /// The fixture's DM must reach the ingress as a DM, bound to «Eval Coach».
    ///
    /// The Slack transport reads `channel_type: "im"` and nothing else as a
    /// 1:1. A DM body without it lands on the room path: the ingress creates
    /// a one-member group for the "room", forges the thread with no agent,
    /// and `first_reply_introduction` asks a coach-less thread to give a
    /// coach's name, which no reply can pass (carnet#894). The transport is
    /// the production one, so the classification asserted here is the one
    /// the live lane gets; the agent a DM with no selection falls back to is
    /// the name that episode requires.
    #[tokio::test]
    #[serial]
    async fn the_fixtures_dm_is_a_direct_message_bound_to_the_eval_coach() {
        let resources = create_test_server_resources().await.unwrap();
        let fixture = seed_fixture(&resources).await;
        let transport = SlackTransport::new("secret".to_owned());

        for (channel, expect_dm) in [(&fixture.dm_channel, true), (&fixture.group_channel, false)] {
            let body = fixture.slack_message_event(channel, "bonjour").to_string();
            let messages = transport
                .parse_inbound(&HeaderMap::new(), body.as_bytes())
                .await
                .unwrap();
            assert_eq!(messages.len(), 1, "{channel}: one inbound message");
            assert_eq!(
                messages[0].is_direct_message,
                expect_dm,
                "{channel}: the transport must read this body as {}",
                if expect_dm { "a DM" } else { "a room message" }
            );
        }

        let bound = selected_or_system_agent(
            &resources.common.repos,
            fixture.athlete_tenant,
            fixture.athlete,
        )
        .await
        .expect("a DM with no selection falls back to the tenant's system agent");
        let agent = resources
            .common
            .repos
            .agents
            .get_by_id(&bound, fixture.athlete, fixture.athlete_tenant)
            .await
            .unwrap()
            .expect("the bound agent exists");
        let required = CORPUS
            .iter()
            .find(|episode| episode.name == "first_reply_introduction")
            .and_then(|episode| episode.turns.first())
            .and_then(|turn| {
                turn.expect.iter().find_map(|expect| match expect {
                    Expect::NamesAgent(title) => Some(*title),
                    _ => None,
                })
            })
            .expect("the introduction episode names the coach it must hear");
        assert_eq!(
            agent.title, required,
            "the DM is bound to «{}» but the introduction episode listens for «{required}»",
            agent.title
        );
    }

    /// The fixture's DM must open as an ordinary coaching thread whose coach
    /// may draw.
    ///
    /// The thread is forged exactly as the Slack session forges a fresh DM.
    /// An athlete with intake outstanding or an empty dossier gets an
    /// interview on it (intake, then the pillars walk), and an interview owns
    /// its turns: it withholds the visual contract, so the chart episodes
    /// grade onboarding instead of the incident. A coach whose `visuals`
    /// grant is empty is never taught the contract either, and answers
    /// «je ne peux pas dessiner de graphique ici» on every chart turn.
    #[tokio::test]
    #[serial]
    async fn the_fixtures_dm_is_a_coaching_thread_that_may_draw() {
        let resources = create_test_server_resources().await.unwrap();
        let fixture = seed_fixture(&resources).await;
        let repos = &resources.common.repos;
        let user_id = fixture.athlete.to_string();

        let conversation_id = forge_conversation(
            repos,
            ForgeParams {
                user_id: &user_id,
                tenant_id: fixture.athlete_tenant,
                title_fallback: "DM",
                model: None,
                agent: ForgeAgent::Selected,
                group_id: None,
                channel_type: "slack",
                selection_source: AgentSelectionSource::MessagingSession,
                guided_flow: true,
                quota: SlotQuota::Exempt,
            },
        )
        .await
        .unwrap();
        let conversation = repos
            .chat
            .get_conversation(&conversation_id, &user_id, fixture.athlete_tenant)
            .await
            .unwrap()
            .expect("the forged DM exists");

        assert_eq!(
            conversation.onboarding_state, None,
            "a fresh fixture DM opened an onboarding interview; every DM episode would \
             grade the interview instead of the incident"
        );
        let agent_id = conversation
            .agent_id
            .expect("a DM with no selection falls back to the tenant's system agent");
        let runtime = repos
            .agents
            .get_agent_runtime_context(&agent_id, fixture.athlete_tenant)
            .await
            .unwrap()
            .expect("the bound agent has a runtime context");
        assert!(
            granted_visuals(Some(&runtime.visuals)).contains(&"chart".to_owned()),
            "the DM's coach may not draw a chart (visuals {:?}); the chart episodes would \
             grade a refusal",
            runtime.visuals
        );
    }

    /// The lane must wait out the server's turn watchdog before it moves on,
    /// wherever an operator sets it.
    ///
    /// A turn still running when the lane gives up delivers its reply after
    /// the next inbound, and the lane reads it as that turn's answer: a
    /// finding filed against the wrong question, or a regression hidden
    /// behind the previous turn's good reply. With the runtime fallback armed
    /// one turn can outlast every per-tier cap — a `copilot_sdk` turn that
    /// times out re-runs on `claude_code` — so the wait is pinned to the one
    /// bound every tier runs under. The 1500 s case is above the watchdog's
    /// default, so a wait pinned to a constant instead of the live ceiling
    /// fails it.
    #[test]
    #[serial]
    fn the_lane_outwaits_the_servers_turn_watchdog() {
        const WATCHDOG_VAR: &str = "MESSAGING_TURN_WATCHDOG_SECS";
        let saved = env::var(WATCHDOG_VAR).ok();
        let observed: Vec<(Option<&str>, u64, u64)> = [None, Some("1500"), Some("60")]
            .into_iter()
            .map(|setting| {
                match setting {
                    Some(secs) => env::set_var(WATCHDOG_VAR, secs),
                    None => env::remove_var(WATCHDOG_VAR),
                }
                (setting, turn_watchdog().as_secs(), turn_timeout().as_secs())
            })
            .collect();
        match saved {
            Some(secs) => env::set_var(WATCHDOG_VAR, secs),
            None => env::remove_var(WATCHDOG_VAR),
        }

        for (setting, watchdog, wait) in observed {
            assert!(
                wait > watchdog,
                "with {WATCHDOG_VAR}={setting:?} the server closes a turn after {watchdog}s \
                 but the lane stops waiting after {wait}s — a turn that runs that long lands \
                 its reply on the next turn and is graded against the wrong question"
            );
        }
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
