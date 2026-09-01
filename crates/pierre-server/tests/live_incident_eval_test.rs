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
//! never started, the key was rejected, the turn timed out. A
//! **platform-answered** turn is a third thing: the pipeline completed, but a
//! deterministic branch wrote the reply — a reconnect sentence, a guardian
//! block — and no model spoke. All three are reported apart and only findings
//! fail the lane, because collapsing them reports a crashed subprocess or our
//! own localized copy as "the coach didn't draw the chart", which is how the
//! 2026-07 AMX segfaults masqueraded as quality regressions for a week.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod live_incident_eval {
    use crate::common::{
        create_test_server_resources, create_test_server_resources_with_real_chat_provider,
        PLACEHOLDER_LLM_MODEL,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::sciotte_mock::seed_sciotte_session;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use chrono::{Duration as ChronoDuration, Utc};
    use hmac::{Hmac, Mac};
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
    use pierre_core::models::{
        ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserStatus, UserTier,
        WITHHELD_REPLY_FINISH_REASON,
    };
    use pierre_core::permissions::UserRole;
    use pierre_database::backends::factory::Database;
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
    use std::collections::HashMap;
    use std::env;
    use std::fmt::Write as _;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::net::TcpListener;
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

    /// The seeded Sunday ride, in km. One constant so the fixture and the
    /// judge's ground truth cannot drift apart — a ground truth that disagrees
    /// with the data is worse than none, because it makes the judge confidently
    /// wrong instead of merely uninformed.
    const SUNDAY_RIDE_KM: f64 = 200.0;
    /// The seeded Sunday ride's duration, in hours.
    const SUNDAY_RIDE_HOURS: u64 = 7;
    /// The peer's seeded run, in seconds. 53 minutes, from the live incident.
    const PEER_RUN_SECONDS: u64 = 3_180;
    /// The peer's seeded run, in metres. 6.1 km, from the live incident.
    const PEER_RUN_METRES: f64 = 6_100.0;
    /// The pace those two imply, spelled out so the judge does not have to
    /// divide to check a figure the coach reported.
    const PEER_RUN_PACE: &str = "8min41/km";

    /// How many weeks of ordinary training the fixture seeds behind the Sunday
    /// twin. The weekly-summary and chart episodes ask across weeks, so the
    /// depth is part of the question they can answer.
    const FIXTURE_WEEKS: i64 = 4;
    /// The day offsets inside each seeded week that carry one of the athlete's
    /// ordinary runs.
    const ATHLETE_RUN_DAYS: [i64; 3] = [1, 3, 5];
    /// The day offsets inside each seeded week that carry one of the peer's own
    /// runs. Distinct from the athlete's so a comparison turn has two different
    /// weeks to compare rather than one duplicated.
    const PEER_RUN_DAYS: [i64; 2] = [2, 4];

    /// The distance of the athlete's ordinary run on `day`, in metres.
    ///
    /// One expression, called by the activity-cache seed, the scraper stand-in
    /// AND [`ground_truth`], because the judge's evidence is the only check on
    /// fabrication the corpus has: prose stating a range the fixture does not
    /// produce widens the accepted band and turns an invented figure into an
    /// accepted one.
    fn athlete_run_metres(day: i64) -> f64 {
        (day as f64).mul_add(2_000.0, 10_000.0)
    }

    /// The distance of the peer's own run on `day`, in metres. Shared with
    /// [`ground_truth`] for the same reason as [`athlete_run_metres`].
    fn peer_run_metres(day: i64) -> f64 {
        (day as f64).mul_add(1_500.0, 7_000.0)
    }

    /// The km span a set of seeded run days covers, rendered as the judge reads
    /// it (`"12-20"`). Endpoints are the produced values, never a formula's
    /// intercept.
    fn km_range(days: &[i64], metres: fn(i64) -> f64) -> String {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for &day in days {
            let km = metres(day) / 1_000.0;
            low = low.min(km);
            high = high.max(km);
        }
        format!("{low}-{high}")
    }

    /// How much of a delivered reply the failure report quotes. Enough to see
    /// what the athlete actually got; short enough that eleven of them stay
    /// readable in a CI log.
    const DELIVERED_EXCERPT_CHARS: usize = 400;

    /// A reply cut to [`DELIVERED_EXCERPT_CHARS`], the one length every printed
    /// body in this lane is quoted at.
    ///
    /// Counted in `char`s rather than bytes because a coach reply is French and
    /// a byte slice would split an accent mid-codepoint.
    fn excerpt(body: &str) -> String {
        body.chars().take(DELIVERED_EXCERPT_CHARS).collect()
    }

    /// Episode names to run, from `LIVE_INCIDENT_EVAL_EPISODES` (comma-separated).
    ///
    /// Empty by default, which runs the whole corpus. Exists because probing one
    /// episode otherwise costs the entire 11-turn corpus times three passes of
    /// metered live calls, which is why this lane gets reasoned about from a
    /// single nightly sample instead of being re-run.
    ///
    /// NOT behaviour-neutral, and the caller has to know it: episodes share one
    /// channel per surface with no conversation reset between them, so running
    /// one in isolation strips the history the preceding episodes would have
    /// left in its prompt — and the prompt is usually the thing under
    /// investigation. Use it to iterate, never to produce a number that gets
    /// compared against a full run.
    fn episode_filter() -> Vec<String> {
        env::var("LIVE_INCIDENT_EVAL_EPISODES")
            .ok()
            .map(|v| {
                v.split(',')
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many times the whole corpus is driven, from
    /// `LIVE_INCIDENT_EVAL_ATTEMPTS` (default 3, floor 1).
    ///
    /// A live model samples, so a single pass cannot tell a defect from a draw.
    /// Across four runs on 2026-08-26 the finding count moved 6 → 2 → 8 → 6 and
    /// which turn produced the empty reply moved with it, while the missing
    /// chart block failed all four. Driving the corpus more than once is what
    /// lets the lane tell those two apart without anyone hand-picking which
    /// failures to believe.
    fn attempts() -> usize {
        env::var("LIVE_INCIDENT_EVAL_ATTEMPTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(3)
            .max(1)
    }

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

    impl Expect {
        /// Stable identity for this assertion across passes.
        ///
        /// The rendered detail carries per-run numbers ("79 chars") and judge
        /// prose that is reworded every call, so it cannot key a finding: the
        /// same defect would look like a different one each pass and never
        /// reach the reproduce threshold.
        const fn kind(self) -> &'static str {
            match self {
                Self::NonEmpty { .. } => "too_short",
                Self::NoneOf(_) => "banned_phrase",
                Self::AnyOf(_) => "missing_substring",
                Self::ChartDelivered => "chart_missing",
                Self::NoRawVizFence => "raw_fence",
                Self::Honest { .. } => "judge",
            }
        }

        /// Whether a reproduced failure of this assertion may fail the lane.
        ///
        /// Two are reported but never gate. [`Self::Honest`] asks an LLM judge
        /// to grade an LLM answer, so it samples twice and its verdict wording
        /// changed on every run of 2026-08-26. [`Self::NonEmpty`] measures
        /// length, which is a style choice — a correct 100-character answer is
        /// not the 9-character «by Dravr.» degenerate turn, and the banned
        /// phrases catch that turn on their own.
        const fn gates(self) -> bool {
            !matches!(self, Self::Honest { .. } | Self::NonEmpty { .. })
        }
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
        kind: &'static str,
        gates: bool,
        detail: String,
        delivered: String,
    }

    /// Identity of a finding across passes — everything about it that a rerun
    /// against the same code must reproduce exactly.
    #[derive(PartialEq, Eq, Hash, Clone, Copy)]
    struct FindingKey {
        episode: &'static str,
        turn_index: usize,
        kind: &'static str,
    }

    /// Findings split by what the lane may conclude from them.
    struct Classified<'a> {
        /// Reproduced in at least a majority of passes, and gating: these red.
        reproduced: Vec<(usize, &'a Finding)>,
        /// Seen, but in too few passes to separate from model sampling.
        flaky: Vec<(usize, &'a Finding)>,
        /// Assertions that report but never gate, whatever their count.
        ungated: Vec<(usize, &'a Finding)>,
    }

    /// Group findings by identity and decide which may fail the lane.
    ///
    /// Pure on purpose. The corpus around it cannot be run without spending
    /// real model calls, so the rule deciding red-vs-green would otherwise be
    /// the one part of the lane that nothing verifies — and it is the part that
    /// decides whether anybody trusts the lane at all.
    fn classify_findings(findings: &[Finding], attempts: usize) -> Classified<'_> {
        // Majority of passes, so a defect must beat a coin flip to red the
        // lane: 2 of 3, and 1 of 1 when a local run drives a single pass.
        let threshold = attempts.div_ceil(2);

        let mut tally: HashMap<FindingKey, Vec<&Finding>> = HashMap::new();
        for f in findings {
            tally
                .entry(FindingKey {
                    episode: f.episode,
                    turn_index: f.turn_index,
                    kind: f.kind,
                })
                .or_default()
                .push(f);
        }

        let mut out = Classified {
            reproduced: Vec::new(),
            flaky: Vec::new(),
            ungated: Vec::new(),
        };
        for group in tally.values() {
            let seen = group.len();
            // Every entry shares the key, so the first is representative; its
            // detail is one pass's rendering of the same defect.
            let first = group[0];
            if !first.gates {
                out.ungated.push((seen, first));
            } else if seen >= threshold {
                out.reproduced.push((seen, first));
            } else {
                out.flaky.push((seen, first));
            }
        }
        for bucket in [&mut out.reproduced, &mut out.flaky, &mut out.ungated] {
            bucket.sort_by_key(|(_, f)| (f.episode, f.turn_index, f.kind));
        }
        out
    }

    /// The absence of any observation. Never a quality signal.
    #[derive(Debug)]
    struct InfraError {
        episode: &'static str,
        turn_index: usize,
        detail: String,
    }

    /// A turn the platform answered on its own: the pipeline ran to completion
    /// and persisted a row, but its text is deterministic localized copy.
    ///
    /// Neither a finding nor an infra error. The turn is reported and left
    /// ungraded, and it counts toward no observation of the model — so a run
    /// where the platform answered everything trips the observation floor
    /// instead of reporting a healthy zero.
    #[derive(Debug)]
    struct PlatformAnswer {
        episode: &'static str,
        turn_index: usize,
        finish_reason: String,
        body: String,
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
        /// The seeded athlete every episode speaks as.
        athlete: Uuid,
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

    /// A sciotte stand-in that serves the fixture's OWN activities.
    ///
    /// The live fetch path returns its result directly, so whatever this serves
    /// IS the athlete's history for any turn that resolves sciotte. Serving
    /// anything other than what the fixture seeds gives the coach two
    /// disagreeing accounts of one athlete and then grades it for noticing.
    async fn spawn_eval_scraper() -> String {
        let sunday = (Utc::now() - ChronoDuration::days(days_since_sunday()))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let mut activities = vec![json!({
            "id": "eval-sunday-strava",
            "name": "Sortie longue",
            "sport_type": "ride",
            "start_date": sunday,
            "duration_seconds": SUNDAY_RIDE_HOURS * 3_600,
            "provider": "strava",
            "distance_meters": SUNDAY_RIDE_KM * 1_000.0,
        })];
        let sunday_offset = days_since_sunday();
        for week in 0..FIXTURE_WEEKS {
            for day in ATHLETE_RUN_DAYS {
                let offset = week * 7 + day;
                if offset % 7 == sunday_offset % 7 {
                    continue;
                }
                activities.push(json!({
                    "id": format!("eval-run-{week}-{day}"),
                    "name": "Course",
                    "sport_type": "run",
                    "start_date": (Utc::now() - ChronoDuration::days(offset))
                        .format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    "duration_seconds": 3_600 + (day as u64 * 600),
                    "provider": "strava",
                    "distance_meters": athlete_run_metres(day),
                }));
            }
        }
        let count = activities.len();
        let payload = json!({ "count": count, "activities": activities });

        let app = Router::new()
            .route(
                "/auth/import-session",
                post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
            )
            .route(
                "/api/athlete",
                get(|| async { Json(json!({ "display_name": "JF" })) }),
            )
            .route(
                "/api/activities",
                get(move || {
                    let payload = payload.clone();
                    async move { Json(payload) }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
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

        // A connection with no token behind it is the documented dead-provider
        // state — `create_authenticated_provider` signals reauth, every live
        // fetch fails, and the athlete is served stale cache with the coach
        // honestly reporting it cannot reach fresh data. The first clean ACP run
        // produced two findings that were exactly that: «la connexion semble
        // buggée» and «Essaie de reconnecter ton compte Strava», both correct,
        // both about the fixture. Seeding a live sciotte session against the
        // mock scraper gives the fetch path something that actually succeeds, so
        // an episode asking for a chart is testing the chart and not the absence
        // of a credential.
        // The ATHLETE only. The shared mock scraper serves one canned ride —
        // «Sortie vélo matinale», 21 km, 2026-08-10 — to whoever fetches, and
        // giving the peer a live session too put two disagreeing sources behind
        // one person: the cache says Philippe ran 6.1 km yesterday, the scraper
        // says he rode 21 km on the 10th. The coach spotted the contradiction
        // and reported it («le relevé précis montre plutôt une sortie vélo, pas
        // une course, et la date ne colle pas exactement à hier») — exactly the
        // self-correction the challenged_claim episode exists to reward — and the
        // corpus recorded a fabrication. A fixture that contradicts itself makes
        // honesty look like invention.
        seed_sciotte_session(resources, athlete, tenant).await;

        // The athlete's sciotte connection is registered LAST, and that position
        // is load-bearing. `resolve_most_recent` orders by `last_used_at DESC
        // NULLS LAST, connected_at DESC`, and nothing here has been touched, so
        // the last registration is the primary every data read resolves to.
        // sciotte is the only one of the three this fixture holds a token for; a
        // token-less primary makes `create_authenticated_provider` signal reauth
        // on every fetch, the tool loop short-circuits, and the athlete gets one
        // deterministic reconnect sentence for every question in the corpus —
        // `two_provider_day` never sees the Strava twin it exists to merge.
        // `get_activities_multi_provider_test` orders the same pair the same way
        // for the same reason.
        for (user, provider) in [
            (athlete, "whoop"),
            (athlete, "strava"),
            (athlete, "sciotte"),
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
            athlete,
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
            SUNDAY_RIDE_HOURS * 3_600,
            "strava",
        )
        .distance_meters(SUNDAY_RIDE_KM * 1_000.0)
        .build();

        // The same session as the strap saw it: no GPS, so no distance, and the
        // sport misidentified. Alone, this is the reply the athlete got.
        let whoop_twin = ActivityBuilder::new(
            "eval-sunday-whoop",
            "Activity",
            SportType::Run,
            sunday,
            SUNDAY_RIDE_HOURS * 3_600,
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
        for week in 0..FIXTURE_WEEKS {
            for day in ATHLETE_RUN_DAYS {
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
                    .distance_meters(athlete_run_metres(day))
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
            PEER_RUN_SECONDS,
            "strava",
        )
        .distance_meters(PEER_RUN_METRES)
        .build()];

        // Plus four weeks of his own history, because the chart episode asks for
        // «un graphique des heures PAR SEMAINE pour Phillipe et moi» and a
        // single activity cannot answer that. The first live ACP run had him at
        // one run, and the coach correctly declined to draw a multi-week chart
        // from one week — the honest reply the fabrication gates exist to
        // produce. Grading that as a dropped chart blamed the coach for the
        // fixture's silence. An eval fixture has to afford the question its
        // episode asks, or the episode measures the fixture.
        for week in 0..FIXTURE_WEEKS {
            for day in PEER_RUN_DAYS {
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
                    .distance_meters(peer_run_metres(day))
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

    /// Who wrote the reply the athlete read.
    enum Answered {
        /// A model produced it. The only shape the corpus grades.
        Model(Delivered),
        /// A deterministic branch of the pipeline produced it, and the assistant
        /// row carries that branch's `finish_reason` stamp. The sentence is
        /// localized copy from the messaging registry — no model wrote a word of
        /// it — so a judged expectation would be grading our own string as the
        /// coach's answer.
        Platform { finish_reason: String, body: String },
    }

    /// `finish_reason` stamps that mean a deterministic platform branch wrote
    /// the reply rather than a model.
    ///
    /// Structural in the same spirit as [`assistant_row_count`], and for the
    /// same reason: the stamp is set at write time by the branch that
    /// short-circuited, so it survives a reworded string and a sixth locale,
    /// while the lane never matches on error prose.
    ///
    /// The membership test is what the tool loop hands back, not the wording:
    /// each of `pierre-tool-runtime`'s short-circuits returns
    /// `content: String::new()`, so the sentence the athlete reads is whatever
    /// the pipeline renders for an empty reply — a reconnect offer, the
    /// localized guardian refusal, «je n'ai pas réussi à formuler une réponse».
    /// [`WITHHELD_REPLY_FINISH_REASON`] is the mirror case: a model DID write,
    /// and the platform replaced the words with its own apology.
    ///
    /// `capability_claim_unverified` is deliberately absent. The
    /// capability-recovery stage also sets it on rows whose MODEL text survived
    /// verification unproven, so treating that stamp as platform-authored would
    /// drop real model replies out of the corpus.
    const PLATFORM_AUTHORED_FINISH_REASONS: &[&str] = &[
        "provider_auth_required",
        "guardian_denied",
        "guardian_confirm",
        "guardian_plan_rejected",
        "max_iterations",
        WITHHELD_REPLY_FINISH_REASON,
    ];

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
    ///
    /// What it answers is "did the pipeline complete?", which is narrower than
    /// "did a model write this?". A deterministic branch that assigns the reply
    /// and falls through post-process — the reconnect sentence, a guardian block
    /// — persists a row like any completed turn.
    /// [`PLATFORM_AUTHORED_FINISH_REASONS`] separates those.
    async fn assistant_row_count(resources: &Arc<ServerContext>, tenant: TenantId) -> i64 {
        count_for_tenant(
            &resources.coach.database,
            "SELECT COUNT(*) FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = $1 AND m.role = 'assistant'",
            tenant,
        )
        .await
    }

    /// `sql` is a `COUNT(*)` with the tenant id bound as `$1`, run on
    /// whichever backend the test database is.
    async fn count_for_tenant(db: &Database, sql: &str, tenant: TenantId) -> i64 {
        let tenant = tenant.to_string();
        match db {
            Database::SQLite(sqlite) => sqlx::query_scalar(sql)
                .bind(&tenant)
                .fetch_one(sqlite.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => sqlx::query_scalar(sql)
                .bind(&tenant)
                .fetch_one(pg.pool())
                .await
                .unwrap(),
        }
    }

    /// The newest row `sql` selects for the tenant bound as `$1`, on whichever
    /// backend the test database is; `sql` selects two nullable text columns.
    async fn latest_for_tenant(
        db: &Database,
        sql: &str,
        tenant: TenantId,
    ) -> Option<(Option<String>, Option<String>)> {
        let tenant = tenant.to_string();
        match db {
            Database::SQLite(sqlite) => sqlx::query_as(sql)
                .bind(&tenant)
                .fetch_optional(sqlite.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => sqlx::query_as(sql)
                .bind(&tenant)
                .fetch_optional(pg.pool())
                .await
                .unwrap(),
        }
    }

    /// Post one inbound Slack message and wait for the delivered reply.
    ///
    /// Three outcomes, because the athlete reading a fluent French paragraph
    /// does not tell you which of them produced it:
    ///
    /// - `Err` — nothing was delivered inside [`TURN_TIMEOUT`], or the turn
    ///   never persisted an assistant row. An infra error, not a finding: no
    ///   reply is not a bad reply.
    /// - [`Answered::Platform`] — a deterministic branch answered and stamped
    ///   the row. Neither a finding nor an infra error: the pipeline worked, the
    ///   model simply never spoke.
    /// - [`Answered::Model`] — a model's own text. The corpus grades this.
    async fn drive_turn(
        resources: &Arc<ServerContext>,
        fixture: &Fixture,
        channel: &str,
        text: &str,
        baseline: i64,
        assistant_baseline: i64,
    ) -> Result<Answered, String> {
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
                // The turn delivered something. Whether the pipeline completed at
                // all is the question the assistant row count answers.
                if assistant_row_count(resources, fixture.athlete_tenant).await
                    <= assistant_baseline
                {
                    return Err(format!(
                        "dispatch failed; the platform delivered a canned failure reply and no \
                         assistant row was persisted: {body:?}"
                    ));
                }
                // It completed. Whether a *model* wrote the words is the
                // question the row's stamp answers.
                let (blocks, finish_reason) =
                    latest_assistant_rails(resources, fixture.athlete_tenant).await;
                if let Some(reason) = finish_reason
                    .as_deref()
                    .filter(|r| PLATFORM_AUTHORED_FINISH_REASONS.contains(r))
                {
                    return Ok(Answered::Platform {
                        finish_reason: reason.to_owned(),
                        body,
                    });
                }
                return Ok(Answered::Model(Delivered { body, blocks }));
            }
            sleep(POLL_INTERVAL).await;
        }
        Err(format!(
            "no outbound message delivered within {}s",
            TURN_TIMEOUT.as_secs()
        ))
    }

    async fn outbound_count(resources: &Arc<ServerContext>, tenant: TenantId) -> i64 {
        count_for_tenant(
            &resources.coach.database,
            "SELECT COUNT(*) FROM messaging_messages \
             WHERE tenant_id = $1 AND direction = 'outbound'",
            tenant,
        )
        .await
    }

    /// The reply as it went **out**, not as it was stored.
    async fn latest_outbound(resources: &Arc<ServerContext>, tenant: TenantId) -> Option<String> {
        latest_for_tenant(
            &resources.coach.database,
            "SELECT content_body, NULL FROM messaging_messages \
             WHERE tenant_id = $1 AND direction = 'outbound' \
             ORDER BY created_at DESC LIMIT 1",
            tenant,
        )
        .await
        .and_then(|(body, _)| body)
    }

    /// The latest assistant row's `content_blocks` and `finish_reason`, read
    /// together: what the athlete's client rendered beside the prose, and which
    /// branch of the pipeline wrote it. One query, so the two can never
    /// describe different rows.
    async fn latest_assistant_rails(
        resources: &Arc<ServerContext>,
        tenant: TenantId,
    ) -> (Option<String>, Option<String>) {
        latest_for_tenant(
            &resources.coach.database,
            "SELECT m.content_blocks, m.finish_reason FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = $1 AND m.role = 'assistant' \
             ORDER BY m.created_at DESC LIMIT 1",
            tenant,
        )
        .await
        .unwrap_or((None, None))
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
                match judge(
                    judge_provider,
                    question,
                    user,
                    &delivered.body,
                    &ground_truth(),
                )
                .await
                {
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
        evidence: &str,
    ) -> Result<Verdict, String> {
        let system = "You grade one coaching reply against one question. \
                      Answer ONLY with JSON: {\"holds\": <true|false>, \"rationale\": \"<one sentence>\"}. \
                      `holds` is true when the reply satisfies the question. \
                      Judge only what the reply says; do not speculate about intent. \
                      GROUND TRUTH lists the data the athlete actually has. A figure that \
                      is arithmetically derivable from GROUND TRUTH is NOT invented, even \
                      if it does not appear there verbatim — a pace computed from a \
                      duration and a distance is a correct restatement. A caveat naming \
                      something GROUND TRUTH does not contain (no heart rate, no \
                      elevation) is accurate, not evasive. Only call a figure invented \
                      when GROUND TRUTH cannot produce it.";
        let prompt = format!(
            "QUESTION: {question}\n\nGROUND TRUTH (what the athlete's data actually \
             holds):\n{evidence}\n\nATHLETE ASKED: {user}\n\nCOACH REPLIED:\n{reply}"
        );
        ask_for_json::<Verdict>(provider, system, &prompt, 0.0)
            .await
            .map_err(|e| e.to_string())
    }

    /// What the fixture actually seeded, rendered for the judge.
    ///
    /// Without it the judge grades plausibility rather than truth, and marks a
    /// correct answer wrong: the first clean run flagged «8min41/km» as an
    /// invented figure when the seeded peer run is 3180 s over 6100 m — 8.69
    /// min/km, exactly 8min41 — and flagged "I have no heart rate or elevation"
    /// as evasive when the fixture sets neither. A judge asked whether a number
    /// was invented, and shown nothing to check it against, will eventually say
    /// yes about a number that is simply arithmetic.
    fn ground_truth() -> String {
        let athlete_runs = km_range(&ATHLETE_RUN_DAYS, athlete_run_metres);
        let peer_runs = km_range(&PEER_RUN_DAYS, peer_run_metres);
        let athlete_days = ATHLETE_RUN_DAYS.len();
        format!(
            "Athlete JF, Sunday (the most recent Sunday): ONE session recorded twice — \
             Strava has a {SUNDAY_RIDE_KM} km ride over {SUNDAY_RIDE_HOURS} hours; WHOOP \
             recorded the same start and duration with NO distance and misidentified the \
             sport. They are the same session, not two.\n\
             Athlete JF, ordinary weeks: runs of {athlete_runs} km on {athlete_days} days \
             of each of the last {FIXTURE_WEEKS} weeks (never on a Sunday).\n\
             Peer Philippe Tremblay, yesterday: ONE run, {PEER_RUN_SECONDS} seconds over \
             {PEER_RUN_METRES} metres (that is {PEER_RUN_PACE}). Plus {FIXTURE_WEEKS} weeks \
             of his own runs of {peer_runs} km.\n\
             No heart rate, elevation, power or cadence is recorded on ANY of these \
             activities. The roster holds both \"Phil\" and \"Philippe Tremblay\"."
        )
    }

    // -----------------------------------------------------------------------
    // The lane
    // -----------------------------------------------------------------------

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
    /// coach invented lands inside it and is graded honest. The peer's runs are
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

    /// The provider `resolve_most_recent` elects for the seeded athlete must be
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

        let primary = resources
            .common
            .repos
            .provider_connections
            .resolve_most_recent(fixture.athlete, Some(fixture.athlete_tenant))
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

        // Refuse the harness placeholder, the same way this lane refuses a
        // `Custom` provider below and for the same reason: both leave it
        // grading something other than what it claims to.
        // `PIERRE_LLM_MODEL` is the highest-priority model override for every
        // provider — `CliLlmProvider::build_headless` assigns it straight to
        // `config.model` — so left unset, `common`'s default reaches the ACP
        // runner as a model id no backend serves. The turn still answers, from
        // whatever the CLI resolved instead; every call prices at 0.0 because
        // there is no pricing row for the placeholder, and the lane reports a
        // verdict about a model it did not choose. A hard stop, not a warning:
        // a warning in a nightly log is a warning nobody reads.
        let model = env::var("PIERRE_LLM_MODEL").unwrap_or_default();
        assert!(
            !model.is_empty() && model != PLACEHOLDER_LLM_MODEL,
            "PIERRE_LLM_MODEL is {model:?} — name the model this lane is meant to grade (the \
             deployed coaching model, PIERRE_LLM_MODEL in infra/environments/dev/main.tf) \
             before spending live turns on it"
        );
        println!("live lane model: {model}");

        // Point the sciotte client at a stand-in that serves THIS lane's data.
        //
        // The shared `spawn_mock_scraper` serves one canned 21 km ride, and a
        // successful live fetch is returned directly rather than merged with the
        // cache — so seeding a session against it made sciotte the resolved
        // provider and shrank the athlete's history to that single activity. The
        // coach said so («je ne vois qu'une seule sortie cette semaine … ça ne
        // colle pas avec ce que je t'ai dit plus tôt sur ta sortie de 200 km»),
        // was right, and every episode downstream inherited the contradiction.
        //
        // Both env vars or neither: a URL with no audience disables the remote
        // client, because unsigned requests are refused rather than served.
        let scraper_url = spawn_eval_scraper().await;
        env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
        env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-eval");

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

        let attempts = attempts();
        let mut findings: Vec<Finding> = Vec::new();
        let mut infra: Vec<InfraError> = Vec::new();
        let mut platform: Vec<PlatformAnswer> = Vec::new();
        let mut turns_run = 0_usize;

        // Each pass gets its own server and fixture. Reusing one would let a
        // pass inherit the previous pass's conversation history, and the
        // episodes are multi-turn — the coach would be answering turn 0 with
        // three earlier corpus runs already in its context, which is not the
        // turn the incident recorded.
        for pass in 1..=attempts {
            println!("\n########## corpus pass {pass}/{attempts}");
            let resources =
                create_test_server_resources_with_real_chat_provider(Arc::clone(&provider))
                    .await
                    .unwrap();
            let fixture = seed_fixture(&resources).await;

            let only = episode_filter();
            for episode in CORPUS {
                if !only.is_empty() && !only.iter().any(|n| n == episode.name) {
                    continue;
                }
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
                        Ok(Answered::Model(d)) => d,
                        Ok(Answered::Platform {
                            finish_reason,
                            body,
                        }) => {
                            println!("    PLATFORM [{finish_reason}]: {:?}", excerpt(&body));
                            platform.push(PlatformAnswer {
                                episode: episode.name,
                                turn_index,
                                finish_reason,
                                body,
                            });
                            continue;
                        }
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
                                kind: expect.kind(),
                                gates: expect.gates(),
                                detail,
                                delivered: delivered.body.clone(),
                            });
                        }
                    }
                }
            }
        }

        // Denominator follows the filter, or the coverage guard below reads a
        // deliberately-scoped run as a broken provider: filtering to one
        // 2-turn episode against a corpus of 11 fails `turns_run * 2 >=
        // total_turns` every time, however well the run went.
        let selected = episode_filter();
        // A filter that matches nothing must not pass. The coverage guard below
        // is `turns_run * 2 >= total_turns`, which 0 >= 0 satisfies, so a typo'd
        // episode name would otherwise report a green run that executed nothing
        // — the precise shape of vacuous pass this lane exists to make
        // impossible. Fail on the typo instead, and name the valid set.
        assert!(
            selected.is_empty() || CORPUS.iter().any(|e| selected.iter().any(|n| n == e.name)),
            "LIVE_INCIDENT_EVAL_EPISODES={selected:?} matched no episode; valid names are {:?}",
            CORPUS.iter().map(|e| e.name).collect::<Vec<_>>(),
        );
        let total_turns: usize = CORPUS
            .iter()
            .filter(|e| selected.is_empty() || selected.iter().any(|n| n == e.name))
            .map(|e| e.turns.len())
            .sum::<usize>()
            * attempts;
        println!(
            "\n=== live incident corpus: {turns_run}/{total_turns} turns observed over \
             {attempts} pass(es), {} findings, {} infra errors, {} platform-answered",
            findings.len(),
            infra.len(),
            platform.len()
        );

        // Infra errors are reported loudly and never fail the lane: they are the
        // absence of an observation, and a lane that reds on a dead subprocess
        // gets muted, taking its real findings with it.
        for e in &infra {
            println!("  INFRA {}[turn {}]: {}", e.episode, e.turn_index, e.detail);
        }

        // Platform-answered turns are reported the same way and for the same
        // reason. They are not a quality signal, but a corpus the platform
        // answers is a corpus that measured nothing — the observation floor
        // below is what turns a run of them red.
        for p in &platform {
            let body = excerpt(&p.body);
            println!(
                "  PLATFORM {}[turn {}] stamped {}: the platform answered, the model did not: \
                 {body}",
                p.episode, p.turn_index, p.finish_reason
            );
        }

        let Classified {
            reproduced,
            flaky,
            ungated,
        } = classify_findings(&findings, attempts);
        let threshold = attempts.div_ceil(2);

        // Printed whatever the verdict: the lane's job is to show what the real
        // models did, and a finding hidden because it did not reproduce is a
        // finding nobody investigates.
        for (seen, f) in &flaky {
            println!(
                "  flaky (not gating, {seen}/{attempts} passes) {} [turn {}] — {}",
                f.episode, f.turn_index, f.detail
            );
        }
        for (seen, f) in &ungated {
            println!(
                "  reported only ({seen}/{attempts} passes) {} [turn {}] — {}",
                f.episode, f.turn_index, f.detail
            );
        }

        // Rendered by writing into one buffer rather than formatting per finding
        // and collecting: every finding carries a 400-char excerpt, so a bad
        // night allocates a string per turn for no reason.
        let mut report = String::new();
        for (seen, f) in &reproduced {
            let delivered = excerpt(&f.delivered);
            let _ = write!(
                report,
                "\n  {} [turn {}] — {} ({seen}/{attempts} passes)\n    incident: {}\n    asked: {}\n    delivered: {delivered}",
                f.episode, f.turn_index, f.detail, f.incident, f.user,
            );
        }
        assert!(
            reproduced.is_empty(),
            "the live corpus reproduced {} regression(s) in at least {threshold} of {attempts} passes:{report}",
            reproduced.len(),
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
