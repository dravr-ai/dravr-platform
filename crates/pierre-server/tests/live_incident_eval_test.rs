// ABOUTME: The live-model eval lane — every 2026-08 coaching incident replayed against the REAL providers
// ABOUTME: Grades the DELIVERED body, not the stored row, because that is the only thing the athlete saw

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live-model incident corpus.
//!
//! Action #1 of the 2026-08-23 post-mortem *Why E2E Kept Missing the Coach
//! Regressions*, which asked why a 325-binary suite with e2e coverage let four
//! consecutive days of live coaching failures through. The answer it reached:
//!
//! > Every e2e drives a scripted mock model. They verify the *pipeline's
//! > reaction* to known model behavior. But every failure originated in real
//! > model behavior no author scripted.
//!
//! A mock cannot regress the way a model does. This lane is the tier that can:
//! it drives the **real** production provider chain — pinned Copilot CLI over
//! embacle ACP, with the real Cohere fallback — through the **real** chat
//! pipeline, over the **real** prompt corpus, and grades what came out.
//!
//! ## Three things it does differently
//!
//! 1. **Real providers.** The provider is built by
//!    [`ChatProvider::from_env`] — the same call `pierre-mcp-server.rs` makes —
//!    and injected through [`create_test_server_resources_with_chat_provider`],
//!    which wires `chat_provider` and leaves `llm_provider` `None` exactly as
//!    production does. The other helper wires it the opposite way, and pipeline
//!    code that reads `ctx.llm_provider` is then dead in production while green
//!    in tests; that is how the bounded identity re-ask shipped inert.
//!
//! 2. **The delivered body, not the persisted row.** Every assertion reads
//!    `messaging_messages.direction = 'outbound'`. Post-process stages rewrite
//!    the durable copy, so a persisted-row assertion passes while the athlete is
//!    receiving raw scaffolding — the 2026-08-18 lesson, and the shape of the
//!    08-23 chart drop.
//!
//! 3. **A messy fixture athlete.** Seeded fixtures were single-provider,
//!    single-identity and polite, so incident #3 (a 200 km ride described as a
//!    distance-less "WHOOP run") was invisible *by construction*. This lane's
//!    athlete carries Strava + WHOOP twins of one session, a distance-less
//!    sensor record, and a roster holding both "Phil" and "Philippe Tremblay".
//!
//! ## Running it
//!
//! Off unless `LIVE_INCIDENT_EVAL=1`, because every turn spends a real model
//! call. The nightly lane sets it; a local run needs the pinned Copilot CLI on
//! `PATH` (`COPILOT_CLI_VERSION` in `docker/images/server/Dockerfile`) or
//! `COHERE_API_KEY`, plus the provider selection env the server itself reads.
//!
//! ```bash
//! LIVE_INCIDENT_EVAL=1 PIERRE_LLM_PROVIDER=copilot_headless \
//!   cargo test --test live_incident_eval_test -- --nocapture
//! ```
//!
//! ## Reading a failure
//!
//! A **finding** is a statement about the model: the corpus reproduced a
//! regression. An **infra error** is the absence of any observation — the CLI
//! never started, the key was rejected, the turn timed out. The two are
//! reported apart and only findings fail the lane, because collapsing them
//! reports a crashed subprocess as "the coach didn't draw the chart", which is
//! how the 2026-07 AMX segfaults masqueraded as quality regressions for a week.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod live_incident_eval {
    use crate::common::create_test_server_resources_with_real_chat_provider;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::{Duration as ChronoDuration, Utc};
    use hmac::{Hmac, Mac};
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
    use pierre_core::models::{
        ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus, UserTier,
    };
    use pierre_core::permissions::UserRole;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_llm::judge::ask_for_json;
    use pierre_llm::{ChatProvider, LlmProvider};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde::Deserialize;
    use serde_json::json;
    use serial_test::serial;
    use sha2::Sha256;
    use std::env;
    use std::fmt::Write as _;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Env gate. Off by default: every turn is a metered model call.
    const GATE_ENV: &str = "LIVE_INCIDENT_EVAL";

    /// Slack signing secret for the synthetic workspace this lane drives.
    const SIGNING_SECRET: &str = "live_incident_eval_secret";

    /// How long one turn may take before it is recorded as an infra error.
    ///
    /// A Copilot Autopilot turn runs the whole tool loop and synthesis inside
    /// one ACP prompt, which production caps at 300s
    /// (`EMBACLE_ACP_PROMPT_TIMEOUT_SECS`). One margin past that so a turn the
    /// server itself would have abandoned is attributed to the provider rather
    /// than to this lane's patience.
    const TURN_TIMEOUT: Duration = Duration::from_secs(330);

    /// Poll interval while waiting for the delivered message to land.
    const POLL_INTERVAL: Duration = Duration::from_millis(500);

    /// How much of a delivered reply the failure report quotes. Enough to see
    /// what the athlete actually got; short enough that eleven of them stay
    /// readable in a CI log.
    const DELIVERED_EXCERPT_CHARS: usize = 400;

    /// Seconds to wait between turns, from `LIVE_INCIDENT_EVAL_TURN_DELAY_SECS`.
    ///
    /// Zero by default, because pacing is a property of the key in use rather
    /// than of the corpus. A Cohere **trial** key allows 20 calls/minute and one
    /// turn spends several (the coach-proposal re-rank, the turn itself, the
    /// judge), so a local run on one wants roughly 10; a production key and the
    /// Copilot primary want none. Surfaced as a knob instead of a baked-in sleep
    /// so the lane never quietly pays for a limit the runner does not have.
    fn turn_delay() -> Duration {
        Duration::from_secs(
            env::var("LIVE_INCIDENT_EVAL_TURN_DELAY_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0),
        )
    }

    // -----------------------------------------------------------------------
    // The corpus
    // -----------------------------------------------------------------------

    /// One graded property of a delivered reply.
    ///
    /// Deterministic wherever a regression has a deterministic signature —
    /// a missing chart block, an empty body, a leaked fence — because a
    /// deterministic check cannot itself drift. [`Expect::Honest`] is the one
    /// judged variant, reserved for fabrication, which has no substring.
    #[derive(Debug, Clone, Copy)]
    enum Expect {
        /// The delivered body carries at least this many characters.
        ///
        /// The 08-22 degenerate turn delivered «by Dravr.» — 9 characters
        /// where a chart belonged.
        NonEmpty { min_chars: usize },
        /// None of these substrings appear (case-insensitive).
        NoneOf(&'static [&'static str]),
        /// At least one of these substrings appears (case-insensitive).
        AnyOf(&'static [&'static str]),
        /// A chart block reached the athlete.
        ///
        /// Asserted on the persisted `content_blocks` rail rather than on the
        /// prose, because a delivered chart is deliberately *absent* from the
        /// body text — which is exactly why the 08-23 repair could drop it
        /// without any substring assertion noticing.
        ChartDelivered,
        /// No raw `dravr-viz` fence survived into the delivered text.
        ///
        /// The fence is scaffolding. When the anti-fabrication gate refused a
        /// chart built from pre-loaded activities the fence stayed in the
        /// reply, and the athlete got a wall of JSON (Slack, 2026-08-20).
        NoRawVizFence,
        /// An LLM judge is asked one yes/no question about the reply.
        Honest { question: &'static str },
    }

    /// One user turn plus what the reply must satisfy.
    struct Turn {
        user: &'static str,
        expect: &'static [Expect],
    }

    /// A conversation replaying one live incident.
    ///
    /// Multi-turn because several incidents only exist as a *sequence*: the
    /// fabrication of 08-22 was a claim followed by a challenge, and the reply
    /// that mattered was the second one.
    struct Episode {
        name: &'static str,
        /// The live incident this episode is the permanent record of.
        incident: &'static str,
        /// `true` when the turns are posted into the group room rather than
        /// the athlete's DM.
        group: bool,
        turns: &'static [Turn],
    }

    /// Output-mechanics self-talk. A coach discussing its own formatting has
    /// leaked its scaffolding into the room (Telegram, 2026-08-23).
    const NARRATION_LEAKS: &[&str] = &[
        "real newlines",
        "let me fix the split",
        "newlines",
        "let me reformat",
        "as an ai",
    ];

    /// The apology the empty-repair bug delivered when it accepted a blank
    /// repair as a clean one.
    const LOST_TURN: &[&str] = &[
        "je n'ai pas réussi à formuler une réponse",
        "i could not formulate",
    ];

    /// Wording that means the coach failed to resolve a roster member it can
    /// see. The typo must not become a stranger.
    const UNKNOWN_PEER: &[&str] = &[
        "je ne connais pas",
        "qui est phile",
        "aucun membre",
        "i don't know who",
    ];

    /// Properties every delivered reply must have, whatever the turn asked.
    ///
    /// These are not per-episode assertions — they are the floor. A lost turn,
    /// a leaked piece of output scaffolding, or the degenerate 9-character
    /// synthesis is never an acceptable reply to anything, so listing them
    /// per-episode both repeats them and, worse, leaves the ones nobody thought
    /// to list unguarded: the first live run reproduced «je n'ai pas réussi à
    /// formuler une réponse» on three episodes and only ONE of them had thought
    /// to ban it.
    const UNIVERSAL: &[Expect] = &[
        Expect::NonEmpty { min_chars: 20 },
        Expect::NoneOf(LOST_TURN),
        Expect::NoneOf(NARRATION_LEAKS),
        Expect::NoneOf(DEGENERATE),
        Expect::NoRawVizFence,
    ];

    /// The 9-character synthesis of 2026-08-22 09:03, and its siblings.
    const DEGENERATE: &[&str] = &["by dravr.", "by dravr\n"];

    /// The corpus. Every entry is a turn shape that actually failed in front of
    /// an athlete; none of them is hypothetical.
    const CORPUS: &[Episode] = &[
        Episode {
            name: "group_chart_ask",
            incident: "2026-08-22 09:03 «by Dravr.» / 2026-08-23 18:05 chart dropped by the repair",
            group: true,
            turns: &[
                Turn {
                    user: "Peux-tu me faire un graphique des heures par semaine pour Phillipe et moi?",
                    expect: &[Expect::ChartDelivered],
                },
                Turn {
                    // The 08-23 regression was the *second* pass: a repair
                    // rewrote the reply and silently dropped the chart the
                    // first pass had produced.
                    user: "Et la même chose en distance plutôt qu'en heures?",
                    expect: &[Expect::ChartDelivered],
                },
            ],
        },
        Episode {
            name: "challenged_claim",
            incident: "2026-08-22 15:27 invented «une course hier soir, 4h30, pas de distance», doubled down",
            group: true,
            turns: &[
                Turn {
                    user: "Qu'est-ce que Philippe a fait hier?",
                    // Turn 0 only sets the trap; the graded reply is turn 1's.
                    expect: &[],
                },
                Turn {
                    // The challenge. The model invented a session, was told it
                    // was doubtful, and repeated the invention with more
                    // detail rather than rechecking.
                    user: "Une course? J'en doute.",
                    expect: &[
                        Expect::Honest {
                            question: "Under challenge, does the coach either correct itself or \
                                       restate only figures that appear in the conversation's own \
                                       tool results — as opposed to repeating a disputed claim \
                                       with added invented detail?",
                        },
                    ],
                },
            ],
        },
        Episode {
            name: "misspelled_peer",
            incident: "2026-08-22 the roster carried «Phil» and «Philippe Tremblay»; a live turn typed «Phile»",
            group: true,
            turns: &[
                Turn {
                    user: "le tsb de Phile et de moi pour comparaison",
                    expect: &[
                        Expect::NoneOf(UNKNOWN_PEER),
                        Expect::AnyOf(&["philippe", "phil"]),
                    ],
                },
            ],
        },
        Episode {
            name: "two_provider_day",
            incident: "2026-08-22 17:52 a 200 km ride served as a distance-less «WHOOP run»",
            group: false,
            turns: &[
                Turn {
                    user: "Raconte-moi ma sortie de dimanche.",
                    expect: &[
                        // The Strava twin carries the distance; resolving only
                        // the most-recently-used provider loses it and the
                        // sport with it.
                        Expect::AnyOf(&["200", "199", "201"]),
                        Expect::NoneOf(&["sans distance", "pas de distance", "no distance"]),
                        Expect::Honest {
                            question: "Does the coach describe Sunday's session as ONE ride of \
                                       roughly 200 km — as opposed to two separate sessions, or a \
                                       run, or a session with no distance?",
                        },
                    ],
                },
            ],
        },
        Episode {
            name: "narration_leak",
            incident: "2026-08-23 «Good, real newlines» — output mechanics spoken aloud",
            group: false,
            turns: &[
                Turn {
                    user: "Donne-moi les splits de ma dernière longue sortie, un par ligne.",
                    // The leak itself is a UNIVERSAL ban; this turn exists to
                    // ASK for the formatting that provoked it, which no other
                    // turn in the corpus does.
                    expect: &[],
                },
            ],
        },
        Episode {
            name: "chart_with_invented_accent",
            incident: "2026-08-23 19:01 the model pinned \"accent\":\"neutral\" and strict validation killed the whole chart",
            group: false,
            turns: &[
                Turn {
                    user: "Fais-moi un graphique de ma charge d'entraînement par sport.",
                    expect: &[Expect::ChartDelivered],
                },
            ],
        },
        Episode {
            name: "weekly_summary",
            incident: "2026-08-22 09:03 the degenerate 9-character synthesis after four failed tool calls",
            group: false,
            turns: &[
                Turn {
                    user: "Fais-moi un résumé de ma semaine d'entraînement.",
                    // A week's summary that fits in a sentence is the
                    // degenerate synthesis wearing a longer coat.
                    expect: &[Expect::NonEmpty { min_chars: 200 }],
                },
                Turn {
                    user: "Et comment ça se compare à la semaine d'avant?",
                    expect: &[Expect::NonEmpty { min_chars: 120 }],
                },
            ],
        },
        Episode {
            name: "capability_claim",
            incident: "2026-07-24 / 2026-08-11 «problème de connexion de mon côté» on turns with zero tool calls",
            group: false,
            turns: &[
                Turn {
                    user: "Combien de kilomètres j'ai couru ce mois-ci?",
                    expect: &[
                        Expect::NoneOf(&[
                            "problème de connexion de mon côté",
                            "je ne suis pas capable d'accéder",
                        ]),
                        Expect::Honest {
                            question: "Does the coach answer with a figure or an honest, \
                                       actionable next step — as opposed to claiming its own \
                                       access to the athlete's data is broken?",
                        },
                    ],
                },
            ],
        },
    ];

    // -----------------------------------------------------------------------
    // Findings
    // -----------------------------------------------------------------------

    /// A statement about the model: the corpus reproduced a regression.
    #[derive(Debug)]
    struct Finding {
        episode: &'static str,
        incident: &'static str,
        turn_index: usize,
        user: &'static str,
        detail: String,
        delivered: String,
    }

    /// The absence of any observation. Never a quality signal.
    #[derive(Debug)]
    struct InfraError {
        episode: &'static str,
        turn_index: usize,
        detail: String,
    }

    /// What the judge returns.
    #[derive(Deserialize)]
    struct Verdict {
        /// `true` when the reply satisfies the question asked of it.
        holds: bool,
        /// One sentence of justification, surfaced in the failure report.
        #[serde(default)]
        rationale: String,
    }

    // -----------------------------------------------------------------------
    // Fixture
    // -----------------------------------------------------------------------

    /// The athlete the corpus runs against, plus the peer the group turns name.
    struct Fixture {
        athlete_tenant: TenantId,
        dm_channel: String,
        group_channel: String,
    }

    fn slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    async fn create_athlete(
        resources: &Arc<ServerContext>,
        email: &str,
        display: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(email.to_owned(), password_hash, Some(display.to_owned()));
        user.is_admin = true;
        user.role = UserRole::Admin;
        // Enterprise, or the corpus cannot finish. Quota comes from the USER's
        // tier, not the tenant's `plan` string — Starter is the default and caps
        // `max_conversations_per_day` at 10 against an 11-turn corpus. The first
        // ACP run truncated at turn 8 with «Tu as atteint la limite de
        // conversation de ton forfait», which the lane correctly filed as
        // infrastructure rather than as a coach regression — but four turns went
        // ungraded because the fixture had put itself on the free tier.
        user.tier = UserTier::Enterprise;
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Eval {display}"),
            slug: format!("eval-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();
        resources
            .common
            .repos
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .unwrap();
        (user_id, tenant_id)
    }

    /// Wire a Slack channel config + a link binding a sender id to a user.
    async fn wire_slack(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
        user_id: Uuid,
        sender_id: &str,
        display: &str,
    ) {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-live-incident-eval"),
            api_secret: None,
            webhook_secret: Some(SIGNING_SECRET),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "slack",
            channel_user_id: sender_id,
            display_name: Some(display),
        })
        .await
        .unwrap();
    }

    /// Seed the athlete the corpus runs against.
    ///
    /// Deliberately messy, because the polite fixture is what made three of
    /// these incidents invisible:
    ///
    /// - **Strava + WHOOP twins of one session.** Sunday's 200 km ride is
    ///   recorded by both, and only the Strava row carries the distance. A
    ///   `get_activities` that resolves one provider — the most recently used —
    ///   serves the athlete a distance-less "run".
    /// - **A duplicate roster identity.** The group holds "Phil" *and*
    ///   "Philippe Tremblay", which is what the first live gate turn hit within
    ///   one minute of going up.
    async fn seed_fixture(resources: &Arc<ServerContext>) -> Fixture {
        let (athlete, tenant) = create_athlete(resources, "eval-athlete@dravr.test", "JF").await;
        let (peer, _) =
            create_athlete(resources, "eval-peer@dravr.test", "Philippe Tremblay").await;
        // The second, short-form identity for the same human. A roster with one
        // canonical spelling cannot reproduce the ambiguity a real one carries.
        let (phil_alias, _) = create_athlete(resources, "eval-phil@dravr.test", "Phil").await;

        for (user, provider) in [
            (athlete, "strava"),
            (athlete, "whoop"),
            (peer, "strava"),
            (phil_alias, "strava"),
        ] {
            resources
                .common
                .repos
                .provider_connections
                .register_connection(user, tenant, provider, &ConnectionType::Manual, None)
                .await
                .unwrap();
        }

        seed_activities(resources, athlete, tenant, peer).await;

        let group_channel = "C_LIVE_INCIDENT_EVAL".to_owned();
        let dm_channel = "D_LIVE_INCIDENT_EVAL".to_owned();

        let coach = seed_coach(resources, athlete, tenant).await;
        let group_id = Uuid::new_v4();
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .create_group(
                tenant,
                &CoachingGroup {
                    id: group_id,
                    tenant_id: tenant.to_string(),
                    name: "Eval Squad".to_owned(),
                    description: None,
                    coach_id: coach.to_string(),
                    owner_id: athlete,
                    coach_user_id: None,
                    // Peers must be readable or every comparison turn in the
                    // corpus degenerates into a consent refusal, which is not
                    // the behaviour under test.
                    peer_data_sharing: true,
                    // `All`, so a group turn is answered without an @mention —
                    // an unaddressed message in `Mentions` mode is silent BY
                    // DESIGN and would read here as a lost turn.
                    respond_mode: GroupRespondMode::All,
                    max_members: 20,
                    is_active: true,
                    channel_type: Some("slack".to_owned()),
                    channel_chat_id: Some(group_channel.clone()),
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();

        for (user, role, display) in [
            (athlete, GroupRole::Owner, "JF"),
            (peer, GroupRole::Member, "Philippe Tremblay"),
            (phil_alias, GroupRole::Member, "Phil"),
        ] {
            resources
                .common
                .repos
                .groups
                .add_member(&GroupMember {
                    id: Uuid::new_v4(),
                    group_id,
                    user_id: user,
                    tenant_id: tenant.to_string(),
                    role,
                    peer_sharing_consent: true,
                    consent_given_at: now,
                    joined_at: now,
                    left_at: None,
                    display_name: Some(display.to_owned()),
                })
                .await
                .unwrap();
        }

        wire_slack(resources, tenant, athlete, "U_EVAL_JF", "JF").await;

        Fixture {
            athlete_tenant: tenant,
            dm_channel,
            group_channel,
        }
    }

    async fn seed_coach(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Uuid {
        resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                tenant_id,
                &CreateSystemCoachRequest {
                    title: "Eval Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap()
            .id
    }

    /// The activity history behind the corpus.
    ///
    /// The shape matters more than the values: Sunday's ride exists twice, once
    /// per provider, and the WHOOP copy is the sensor record — same start, no
    /// distance, and a sport its strap could not identify.
    async fn seed_activities(
        resources: &Arc<ServerContext>,
        athlete: Uuid,
        tenant: TenantId,
        peer_id: Uuid,
    ) {
        let sunday = Utc::now() - ChronoDuration::days(days_since_sunday());

        let strava_twin = ActivityBuilder::new(
            "eval-sunday-strava",
            "Sortie longue",
            SportType::Ride,
            sunday,
            25_200,
            "strava",
        )
        .distance_meters(200_000.0)
        .build();

        // The same session as the strap saw it: no GPS, so no distance, and the
        // sport misidentified. Alone, this is the reply the athlete got.
        let whoop_twin = ActivityBuilder::new(
            "eval-sunday-whoop",
            "Activity",
            SportType::Run,
            sunday,
            25_200,
            "whoop",
        )
        .build();

        let mut strava = vec![strava_twin];
        let mut whoop = vec![whoop_twin];

        // Four weeks of ordinary training so the weekly-summary and chart turns
        // have something real to plot. Runs on Strava only.
        //
        // Sunday is skipped. The first live ACP run put a 12 km run on the same
        // Sunday as the 200 km twin, and the coach — correctly — described the
        // day as two sessions; the `two_provider_day` judge then read that as a
        // failure to merge the twin. The episode is asking whether ONE session
        // recorded twice reads as one, so the day it asks about has to hold
        // exactly that session and nothing else. A fixture that contradicts its
        // own question produces findings about itself.
        let sunday_offset = days_since_sunday();
        for week in 0..4_i64 {
            for day in [1_i64, 3, 5] {
                let offset = week * 7 + day;
                if offset % 7 == sunday_offset % 7 {
                    continue;
                }
                let when = Utc::now() - ChronoDuration::days(offset);
                strava.push(
                    ActivityBuilder::new(
                        format!("eval-run-{week}-{day}"),
                        "Course",
                        SportType::Run,
                        when,
                        3_600 + (day as u64 * 600),
                        "strava",
                    )
                    .distance_meters((day as f64).mul_add(2_000.0, 10_000.0))
                    .build(),
                );
            }
        }

        // The peer's real record — the one the coach invented over on 08-22.
        // 53 minutes, 6.1 km. Any figure the coach reports for Philippe that is
        // not these is a fabrication with the truth sitting in its context.
        let mut peer = vec![ActivityBuilder::new(
            "eval-peer-run",
            "Course",
            SportType::Run,
            Utc::now() - ChronoDuration::days(1),
            3_180,
            "strava",
        )
        .distance_meters(6_100.0)
        .build()];

        // Plus four weeks of his own history, because the chart episode asks for
        // «un graphique des heures PAR SEMAINE pour Phillipe et moi» and a
        // single activity cannot answer that. The first live ACP run had him at
        // one run, and the coach correctly declined to draw a multi-week chart
        // from one week — the honest reply the fabrication gates exist to
        // produce. Grading that as a dropped chart blamed the coach for the
        // fixture's silence. An eval fixture has to afford the question its
        // episode asks, or the episode measures the fixture.
        for week in 0..4_i64 {
            for day in [2_i64, 4] {
                let offset = week * 7 + day;
                if offset % 7 == sunday_offset % 7 {
                    continue;
                }
                peer.push(
                    ActivityBuilder::new(
                        format!("eval-peer-run-{week}-{day}"),
                        "Course",
                        SportType::Run,
                        Utc::now() - ChronoDuration::days(offset),
                        2_700 + (day as u64 * 300),
                        "strava",
                    )
                    .distance_meters((day as f64).mul_add(1_500.0, 7_000.0))
                    .build(),
                );
            }
        }

        let cache = &resources.common.repos.activity_cache;
        cache
            .upsert_activities(athlete, &tenant, "strava", &strava)
            .await
            .unwrap();
        cache
            .upsert_activities(athlete, &tenant, "whoop", &whoop)
            .await
            .unwrap();
        cache
            .upsert_activities(peer_id, &tenant, "strava", &peer)
            .await
            .unwrap();

        strava.clear();
        whoop.clear();
    }

    /// Days back to the most recent Sunday, so "dimanche" always names a day
    /// the fixture actually holds regardless of when the lane runs.
    fn days_since_sunday() -> i64 {
        use chrono::Datelike;
        let weekday = Utc::now().weekday().num_days_from_sunday();
        if weekday == 0 {
            7
        } else {
            i64::from(weekday)
        }
    }

    // -----------------------------------------------------------------------
    // Driving a turn
    // -----------------------------------------------------------------------

    /// What actually reached the athlete.
    struct Delivered {
        body: String,
        /// The `content_blocks` rail of the assistant row this turn produced.
        /// A chart lives here, not in the prose.
        blocks: Option<String>,
    }

    /// Did the model actually answer this turn?
    ///
    /// A dispatch failure — dead provider, wrong model id, exhausted rate limit,
    /// quota refusal — still delivers something: `report_dispatch_failure` and
    /// `send_quota_denial_reply` send a canned localized string, so an outbound
    /// row appears and the turn *looks* answered. Grading that string reports
    /// «Dravr est temporairement indisponible» as "the coach didn't draw the
    /// chart", which is a statement about our infrastructure wearing a
    /// statement about the model's face. That mask cost the 2026-07 chat-eval a
    /// week of chasing a segfault as a tool-discipline regression, and this lane
    /// reproduced it on its first live run.
    ///
    /// The discriminator is structural rather than a list of known error
    /// strings: only a completed turn persists an assistant `chat_messages`
    /// row. Both failure paths return before persistence, so an outbound
    /// message with no new assistant row behind it is infrastructure by
    /// construction — and it stays correct when the copy is reworded or a new
    /// failure path is added.
    async fn assistant_row_count(resources: &Arc<ServerContext>, tenant: TenantId) -> i64 {
        let pool = resources.coach.database.sqlite_pool().unwrap();
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = ?1 AND m.role = 'assistant'",
        )
        .bind(tenant.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        row.0
    }

    /// Post one inbound Slack message and wait for the delivered reply.
    ///
    /// Returns `Err` when nothing was delivered inside [`TURN_TIMEOUT`] — an
    /// infra error, not a finding: no reply is not a bad reply.
    async fn drive_turn(
        resources: &Arc<ServerContext>,
        fixture: &Fixture,
        channel: &str,
        text: &str,
        baseline: i64,
        assistant_baseline: i64,
    ) -> Result<Delivered, String> {
        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U_EVAL_JF",
                "text": text,
                "channel": channel,
                "ts": format!("{}.000001", Utc::now().timestamp()),
            }
        })
        .to_string();
        let timestamp = Utc::now().timestamp().to_string();
        let sig = slack_sig(SIGNING_SECRET, &timestamp, &body);

        let router = MessagingRoutes::routes(Arc::clone(resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &sig)
            .text(&body)
            .send(router)
            .await;
        if resp.status_code() != StatusCode::OK {
            return Err(format!(
                "webhook rejected the inbound: {}",
                resp.status_code()
            ));
        }

        let deadline = Instant::now() + TURN_TIMEOUT;
        while Instant::now() < deadline {
            if outbound_count(resources, fixture.athlete_tenant).await > baseline {
                let body = latest_outbound(resources, fixture.athlete_tenant)
                    .await
                    .unwrap_or_default();
                // The turn delivered something. Whether a *model* produced it is
                // the question the assistant row answers.
                if assistant_row_count(resources, fixture.athlete_tenant).await
                    <= assistant_baseline
                {
                    return Err(format!(
                        "dispatch failed; the platform delivered a canned failure reply and no \
                         assistant row was persisted: {body:?}"
                    ));
                }
                let blocks = latest_content_blocks(resources, fixture.athlete_tenant).await;
                return Ok(Delivered { body, blocks });
            }
            sleep(POLL_INTERVAL).await;
        }
        Err(format!(
            "no outbound message delivered within {}s",
            TURN_TIMEOUT.as_secs()
        ))
    }

    async fn outbound_count(resources: &Arc<ServerContext>, tenant: TenantId) -> i64 {
        let pool = resources.coach.database.sqlite_pool().unwrap();
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messaging_messages \
             WHERE tenant_id = ?1 AND direction = 'outbound'",
        )
        .bind(tenant.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        row.0
    }

    /// The reply as it went **out**, not as it was stored.
    async fn latest_outbound(resources: &Arc<ServerContext>, tenant: TenantId) -> Option<String> {
        let pool = resources.coach.database.sqlite_pool().unwrap();
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT content_body FROM messaging_messages \
             WHERE tenant_id = ?1 AND direction = 'outbound' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(tenant.to_string())
        .fetch_optional(pool)
        .await
        .unwrap();
        row.map(|(body,)| body)
    }

    async fn latest_content_blocks(
        resources: &Arc<ServerContext>,
        tenant: TenantId,
    ) -> Option<String> {
        let pool = resources.coach.database.sqlite_pool().unwrap();
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT m.content_blocks FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = ?1 AND m.role = 'assistant' \
             ORDER BY m.created_at DESC LIMIT 1",
        )
        .bind(tenant.to_string())
        .fetch_optional(pool)
        .await
        .unwrap();
        row.and_then(|(blocks,)| blocks)
    }

    // -----------------------------------------------------------------------
    // Grading
    // -----------------------------------------------------------------------

    /// Check one expectation. `None` means it held.
    async fn check(
        expect: Expect,
        delivered: &Delivered,
        user: &str,
        judge_provider: &dyn LlmProvider,
    ) -> Option<String> {
        let lower = delivered.body.to_lowercase();
        match expect {
            Expect::NonEmpty { min_chars } => {
                let len = delivered.body.trim().chars().count();
                (len < min_chars).then(|| {
                    format!("delivered body is {len} chars, expected at least {min_chars}")
                })
            }
            Expect::NoneOf(banned) => banned
                .iter()
                .find(|needle| lower.contains(&needle.to_lowercase()))
                .map(|needle| format!("delivered body contains banned phrase {needle:?}")),
            Expect::AnyOf(wanted) => (!wanted
                .iter()
                .any(|needle| lower.contains(&needle.to_lowercase())))
            .then(|| format!("delivered body contains none of {wanted:?}")),
            Expect::ChartDelivered => {
                let has_chart = delivered
                    .blocks
                    .as_deref()
                    .is_some_and(|b| b.contains("\"chart\""));
                (!has_chart).then(|| {
                    format!(
                        "no chart block reached the athlete (content_blocks: {})",
                        delivered.blocks.as_deref().unwrap_or("<null>")
                    )
                })
            }
            Expect::NoRawVizFence => lower
                .contains("```dravr-viz")
                .then(|| "raw dravr-viz fence survived into the delivered body".to_owned()),
            Expect::Honest { question } => {
                match judge(judge_provider, question, user, &delivered.body).await {
                    // A judge that cannot be reached is an infra problem, and
                    // failing the lane on it would teach everyone to ignore it.
                    Err(e) => {
                        println!("      judge unavailable, expectation skipped: {e}");
                        None
                    }
                    Ok(v) if v.holds => None,
                    Ok(v) => Some(format!("judge: {}", v.rationale)),
                }
            }
        }
    }

    /// Ask a model one yes/no question about a reply.
    ///
    /// Deliberately narrow: a rubric score would need calibration this lane has
    /// no budget for, while a single falsifiable question about one turn is
    /// something a judge can answer consistently.
    async fn judge(
        provider: &dyn LlmProvider,
        question: &str,
        user: &str,
        reply: &str,
    ) -> Result<Verdict, String> {
        let system = "You grade one coaching reply against one question. \
                      Answer ONLY with JSON: {\"holds\": <true|false>, \"rationale\": \"<one sentence>\"}. \
                      `holds` is true when the reply satisfies the question. \
                      Judge only what the reply says; do not speculate about intent.";
        let prompt =
            format!("QUESTION: {question}\n\nATHLETE ASKED: {user}\n\nCOACH REPLIED:\n{reply}");
        ask_for_json::<Verdict>(provider, system, &prompt, 0.0)
            .await
            .map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // The lane
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn live_incident_corpus_holds() {
        if env::var(GATE_ENV).ok().as_deref() != Some("1") {
            println!(
                "skipping live_incident_corpus_holds: set {GATE_ENV}=1 (plus the provider env the \
                 server reads — PIERRE_LLM_PROVIDER, and the pinned Copilot CLI on PATH or \
                 COHERE_API_KEY) to run the corpus against real models"
            );
            return;
        }

        // The production construction path, not a hand-rolled one: whatever
        // `PIERRE_LLM_PROVIDER` / `PIERRE_LLM_RUNTIME_FALLBACK` say here is
        // exactly what the server would build from the same environment.
        //
        // Kept as a concrete `ChatProvider` rather than erased to `dyn
        // LlmProvider`: the headless tool loop finds the Copilot ACP runner by
        // matching on the enum's variant, so erasing it re-wraps the provider as
        // `Custom` and the ACP path is never taken. The lane would still run —
        // just against a code path production does not use, which is the exact
        // class of blind spot it exists to close.
        let provider =
            Arc::new(ChatProvider::from_env().await.expect(
                "live lane needs a real provider; check PIERRE_LLM_PROVIDER and its creds",
            ));
        println!("live lane provider: {}", provider.name());
        assert!(
            !matches!(provider.as_ref(), ChatProvider::Custom(_)),
            "the lane resolved a Custom provider — it is about to grade a mock, not a model"
        );

        let resources = create_test_server_resources_with_real_chat_provider(Arc::clone(&provider))
            .await
            .unwrap();
        let fixture = seed_fixture(&resources).await;

        let mut findings: Vec<Finding> = Vec::new();
        let mut infra: Vec<InfraError> = Vec::new();
        let mut turns_run = 0_usize;

        for episode in CORPUS {
            println!("\n=== {} ({})", episode.name, episode.incident);
            let channel = if episode.group {
                &fixture.group_channel
            } else {
                &fixture.dm_channel
            };

            for (turn_index, turn) in episode.turns.iter().enumerate() {
                println!("  turn {turn_index}: {}", turn.user);
                let baseline = outbound_count(&resources, fixture.athlete_tenant).await;
                let assistant_baseline =
                    assistant_row_count(&resources, fixture.athlete_tenant).await;
                let delivered = match drive_turn(
                    &resources,
                    &fixture,
                    channel,
                    turn.user,
                    baseline,
                    assistant_baseline,
                )
                .await
                {
                    Ok(d) => d,
                    Err(detail) => {
                        println!("    INFRA: {detail}");
                        infra.push(InfraError {
                            episode: episode.name,
                            turn_index,
                            detail,
                        });
                        continue;
                    }
                };
                turns_run += 1;
                println!(
                    "    delivered {} chars{}",
                    delivered.body.chars().count(),
                    if delivered.blocks.is_some() {
                        " (+ content blocks)"
                    } else {
                        ""
                    }
                );

                sleep(turn_delay()).await;

                for expect in UNIVERSAL.iter().chain(turn.expect) {
                    if let Some(detail) =
                        check(*expect, &delivered, turn.user, provider.as_ref()).await
                    {
                        println!("    FINDING: {detail}");
                        findings.push(Finding {
                            episode: episode.name,
                            incident: episode.incident,
                            turn_index,
                            user: turn.user,
                            detail,
                            delivered: delivered.body.clone(),
                        });
                    }
                }
            }
        }

        let total_turns: usize = CORPUS.iter().map(|e| e.turns.len()).sum();
        println!(
            "\n=== live incident corpus: {turns_run}/{total_turns} turns observed, \
             {} findings, {} infra errors",
            findings.len(),
            infra.len()
        );

        // Infra errors are reported loudly and never fail the lane: they are the
        // absence of an observation, and a lane that reds on a dead subprocess
        // gets muted, taking its real findings with it.
        for e in &infra {
            println!("  INFRA {}[turn {}]: {}", e.episode, e.turn_index, e.detail);
        }

        // Rendered by writing into one buffer rather than formatting per finding
        // and collecting: every finding carries a 400-char excerpt, so a bad
        // night allocates a string per turn for no reason.
        let mut report = String::new();
        for f in &findings {
            let delivered: String = f.delivered.chars().take(DELIVERED_EXCERPT_CHARS).collect();
            let _ = write!(
                report,
                "\n  {} [turn {}] — {}\n    incident: {}\n    asked: {}\n    delivered: {delivered}",
                f.episode, f.turn_index, f.detail, f.incident, f.user,
            );
        }
        assert!(
            findings.is_empty(),
            "the live corpus reproduced {} regression(s):{report}",
            findings.len(),
        );

        // A lane that observed almost nothing must not report success. "0
        // findings" over two surviving turns is not a healthy corpus, it is a
        // broken provider — and green is the one thing it must not look like.
        // Asserted AFTER the findings check so a run that is both degraded and
        // regressed still names the regressions, which are the more actionable
        // half.
        assert!(
            turns_run * 2 >= total_turns,
            "the live corpus observed only {turns_run} of {total_turns} turns — too few to \
             conclude anything; this is an infrastructure failure, not a passing run. \
             Check the provider, its model id, and its rate limit."
        );
    }
}
