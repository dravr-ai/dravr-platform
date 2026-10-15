// ABOUTME: The live incident corpus, its fixture athlete, its ground truth and the reproduce-or-flaky classifier
// ABOUTME: Included via `#[path] mod live_incident_corpus;` by the live lane and by the offline guards that check it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

//! What the live incident lane drives and how it decides, shared by its two
//! halves (carnet#805).
//!
//! The lane itself — `tests/live/live_incident_corpus_test.rs` — needs a real
//! model and is built only under `live-e2e`. The guards that keep its fixture,
//! its ground truth and its red-vs-green rule honest run in the default suite
//! (`tests/live_incident_eval_test.rs`), because a ground truth that disagrees
//! with the fixture or a classifier that reds on noise corrupts every live
//! verdict and no live run can see it. Each target uses part of this module,
//! hence the module-level `dead_code` allowance.

use crate::helpers::sciotte_mock::seed_sciotte_session;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use pierre_chat_pipeline::stages::viz_blocks::DEFAULT_VISUALS;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, Pillar, SportType, Tenant, TenantId, User,
    UserStatus, UserTier,
};
use pierre_core::permissions::UserRole;
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::{
    CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::seed_models::SeedAgent;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::messaging_ingress::turn_watchdog;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::intake::{record_steps, STATUS_COMPLETE};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// Slack signing secret for the synthetic workspace this lane drives.
pub const SIGNING_SECRET: &str = "live_incident_eval_secret";

/// How far past the server's turn watchdog the lane keeps waiting: room for
/// the turn to start behind the webhook's acknowledgement and for the
/// watchdog's closing notice to land. The real-Slack lane waits the same two
/// minutes past it (`REPLY_WAIT_SECS` in `messaging_eval_real_slack_test.rs`).
const TURN_WAIT_MARGIN: Duration = Duration::from_secs(120);

/// How long the lane waits for one turn's delivered reply before recording
/// the turn as an infra error.
///
/// Above the server's own ceiling on a messaging turn ([`turn_watchdog`],
/// `MESSAGING_TURN_WATCHDOG_SECS`), read live so an operator who moves the
/// ceiling moves this with it. Every turn has therefore ended — with its
/// reply, or with the watchdog's notice — before the lane posts the next one.
/// A wait below it lets a slow turn keep running past the point the lane
/// gave up on it, and its reply then lands after the next inbound, where the
/// lane reads it as that turn's answer and grades it against the wrong
/// question: an infra delay reported as a coach failure, or a coach failure
/// hidden behind the previous turn's good answer.
///
/// No single tier's cap is the right bound once the runtime fallback is
/// armed: a `copilot_sdk` turn that times out at
/// `EMBACLE_SDK_PROMPT_TIMEOUT_SECS` re-runs on the `claude_code` tier, so
/// one healthy turn can run past every per-tier cap. The watchdog is the one
/// bound every tier and every transport runs under.
pub fn turn_timeout() -> Duration {
    turn_watchdog().saturating_add(TURN_WAIT_MARGIN)
}

/// The seeded Sunday ride, in km. One constant so the fixture and the
/// judge's ground truth cannot drift apart — a ground truth that disagrees
/// with the data is worse than none, because it makes the judge confidently
/// wrong instead of merely uninformed.
pub const SUNDAY_RIDE_KM: f64 = 200.0;
/// The seeded Sunday ride's duration, in hours.
pub const SUNDAY_RIDE_HOURS: u64 = 7;
/// The peer's seeded run, in seconds. 53 minutes, from the live incident.
pub const PEER_RUN_SECONDS: u64 = 3_180;
/// The peer's seeded run, in metres. 6.1 km, from the live incident.
pub const PEER_RUN_METRES: f64 = 6_100.0;
/// The pace those two imply, as the agent reported it in the incident. The
/// judge's ledger computes each run's pace itself; the offline guards check
/// that this is the arithmetic of the two constants above and that the
/// ledger line for the run carries it.
pub const PEER_RUN_PACE: &str = "8min41/km";

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

/// The distance of the athlete's ordinary run on `day`, in metres. Read
/// only through [`athlete_runs`], the one list every reader of the fixture
/// shares.
fn athlete_run_metres(day: i64) -> f64 {
    (day as f64).mul_add(2_000.0, 10_000.0)
}

/// The distance of the peer's own run on `day`, in metres. Read only
/// through [`peer_runs`].
fn peer_run_metres(day: i64) -> f64 {
    (day as f64).mul_add(1_500.0, 7_000.0)
}

/// The name every seeded run carries, in the activity cache, the scraper
/// stand-in and the judge's ledger alike.
pub const SEEDED_RUN_NAME: &str = "Course";

/// The title of the fixture's one system agent, the coach every DM episode
/// talks to and the name `first_reply_introduction` must hear.
pub const EVAL_COACH_TITLE: &str = "Eval Coach";

/// The instant the fixture's activity history is measured back from.
///
/// One per process, so the activity cache, the scraper stand-in and the
/// judge's ledger give every activity the same start, to the second. Each
/// read `Utc::now()` on its own, so the ledger could only give dates: the
/// agent quoted a run's real 09:52 start and the judge, shown no times,
/// graded it invented. Per process rather than per pass because the lane
/// builds the scraper's payload once, before its first pass, and every
/// later pass's cache seed has to match it.
fn fixture_clock() -> DateTime<Utc> {
    static ANCHOR: OnceLock<DateTime<Utc>> = OnceLock::new();
    *ANCHOR.get_or_init(Utc::now)
}

/// When a seeded activity `days_ago` started, on [`fixture_clock`].
pub fn seeded_start(days_ago: i64) -> DateTime<Utc> {
    fixture_clock() - ChronoDuration::days(days_ago)
}

/// One seeded run, as every reader of the fixture sees it.
///
/// The activity-cache seed, the scraper stand-in and [`ground_truth`] all
/// read these lists, so the history the agent is served and the evidence
/// the judge grades it against are one set of rows. When they were three
/// hand-written copies, the judge's copy carried distance ranges and no
/// dates: a correct «12 km en 1h10 hier» for JF read to it as invented,
/// because nothing it was shown put a run of JF's on yesterday.
pub struct SeededRun {
    pub id: String,
    pub days_ago: i64,
    pub seconds: u64,
    pub metres: f64,
}

/// The athlete's ordinary runs: [`ATHLETE_RUN_DAYS`] of each of the last
/// [`FIXTURE_WEEKS`] weeks, Sunday skipped (see `seed_activities`).
pub fn athlete_runs() -> Vec<SeededRun> {
    weekly_runs(
        "eval-run",
        &ATHLETE_RUN_DAYS,
        athlete_run_metres,
        3_600,
        600,
    )
}

/// The peer's runs: yesterday's, the one the agent invented over on 08-22,
/// then [`PEER_RUN_DAYS`] of each of the last [`FIXTURE_WEEKS`] weeks.
pub fn peer_runs() -> Vec<SeededRun> {
    let mut runs = vec![SeededRun {
        id: "eval-peer-run".to_owned(),
        days_ago: 1,
        seconds: PEER_RUN_SECONDS,
        metres: PEER_RUN_METRES,
    }];
    runs.extend(weekly_runs(
        "eval-peer-run",
        &PEER_RUN_DAYS,
        peer_run_metres,
        2_700,
        300,
    ));
    runs
}

fn weekly_runs(
    prefix: &str,
    days: &[i64],
    metres: fn(i64) -> f64,
    base_seconds: u64,
    seconds_per_day: u64,
) -> Vec<SeededRun> {
    let sunday_offset = days_since_sunday();
    let mut runs = Vec::new();
    for week in 0..FIXTURE_WEEKS {
        for &day in days {
            let days_ago = week * 7 + day;
            if days_ago % 7 == sunday_offset % 7 {
                continue;
            }
            runs.push(SeededRun {
                id: format!("{prefix}-{week}-{day}"),
                days_ago,
                seconds: base_seconds + day as u64 * seconds_per_day,
                metres: metres(day),
            });
        }
    }
    runs
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
pub enum Expect {
    /// The delivered body carries at least this many characters.
    ///
    /// The 08-22 degenerate turn delivered «by Dravr.» — 9 characters
    /// where a chart belonged.
    NonEmpty { min_chars: usize },
    /// None of these substrings appear (case-insensitive).
    NoneOf(&'static [&'static str]),
    /// At least one of these substrings appears (case-insensitive).
    AnyOf(&'static [&'static str]),
    /// The reply's opening names the agent with this title, by the rule the
    /// introduction ledger records it with (`reply_names_agent`).
    ///
    /// The ledger's rule rather than a substring, because the two must agree:
    /// a reply the ledger does not count as an introduction is asked for one
    /// again on every later turn, which the athlete sees, whatever a
    /// substring says about the first reply.
    NamesAgent(&'static str),
    /// The reply's opening does not name the agent with this title, by the
    /// same rule: an agent already introduced in the thread that names
    /// itself again has introduced itself twice.
    DoesNotNameAgent(&'static str),
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
    /// The athlete's stored profile carries this FTP once the turn is done.
    ///
    /// The only expectation here that reads STATE rather than text, and
    /// deliberately so. What failed on 2026-09-02 was never the wording:
    /// the agent acknowledged the number — «tu l'as mentionné à 380W plus
    /// tôt» — and hand-computed a threshold from it in prose. It simply
    /// wrote nothing, so the next conversation opened on the same flat "je
    /// n'ai pas accès à tes zones" and the derived zones never existed
    /// (registre#250). A reply that quotes 380 W back perfectly and saves
    /// nothing satisfies every substring assertion anyone could write.
    FtpSaved { watts: u32 },
}

impl Expect {
    /// Stable identity for this assertion across passes.
    ///
    /// The rendered detail carries per-run numbers ("79 chars") and judge
    /// prose that is reworded every call, so it cannot key a finding: the
    /// same defect would look like a different one each pass and never
    /// reach the reproduce threshold.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::NonEmpty { .. } => "too_short",
            Self::NoneOf(_) => "banned_phrase",
            Self::AnyOf(_) => "missing_substring",
            Self::NamesAgent(_) => "agent_not_named",
            Self::DoesNotNameAgent(_) => "agent_reintroduced",
            Self::ChartDelivered => "chart_missing",
            Self::NoRawVizFence => "raw_fence",
            Self::Honest { .. } => "judge",
            Self::FtpSaved { .. } => "physiology_not_saved",
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
    pub const fn gates(self) -> bool {
        !matches!(self, Self::Honest { .. } | Self::NonEmpty { .. })
    }
}

/// One user turn plus what the reply must satisfy.
pub struct Turn {
    pub user: &'static str,
    pub expect: &'static [Expect],
}

/// A conversation replaying one live incident.
///
/// Multi-turn because several incidents only exist as a *sequence*: the
/// fabrication of 08-22 was a claim followed by a challenge, and the reply
/// that mattered was the second one.
pub struct Episode {
    pub name: &'static str,
    /// The live incident this episode is the permanent record of.
    pub incident: &'static str,
    /// `true` when the turns are posted into the group room rather than
    /// the athlete's DM.
    pub group: bool,
    pub turns: &'static [Turn],
}

/// Output-mechanics self-talk. An agent discussing its own formatting has
/// leaked its scaffolding into the room (Telegram, 2026-08-23).
pub const NARRATION_LEAKS: &[&str] = &[
    "real newlines",
    "let me fix the split",
    "newlines",
    "let me reformat",
    "as an ai",
];

/// The apology the empty-repair bug delivered when it accepted a blank
/// repair as a clean one.
pub const LOST_TURN: &[&str] = &[
    "je n'ai pas réussi à formuler une réponse",
    "i could not formulate",
];

/// Wording that means the agent failed to resolve a roster member it can
/// see. The typo must not become a stranger.
pub const UNKNOWN_PEER: &[&str] = &[
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
pub const UNIVERSAL: &[Expect] = &[
    Expect::NonEmpty { min_chars: 20 },
    Expect::NoneOf(LOST_TURN),
    Expect::NoneOf(NARRATION_LEAKS),
    Expect::NoneOf(DEGENERATE),
    Expect::NoRawVizFence,
];

/// The 9-character synthesis of 2026-08-22 09:03, and its siblings.
pub const DEGENERATE: &[&str] = &["by dravr.", "by dravr\n"];

/// The corpus. Every entry is a turn shape that actually failed in front of
/// an athlete; none of them is hypothetical.
pub const CORPUS: &[Episode] = &[
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
    // The FIRST direct-message episode, and it must stay first: a pass
    // runs every DM episode in one conversation, so only the turn that
    // opens it meets an agent not yet introduced there. Group episodes
    // above run in the room, a thread of its own, and do not count.
    // `the_introduction_episode_opens_the_direct_message` holds the order.
    Episode {
        name: "first_reply_introduction",
        incident: "2026-09-21 23:46 the Half Marathon Agent's first reply opened straight into analysis, no name, no role (carnet#501)",
        group: false,
        turns: &[
            Turn {
                user: "Montre-moi mes sorties de la semaine avec le dénivelé.",
                expect: &[
                    // The fixture's DM is bound to the agent seeded as
                    // «Eval Coach» — the tenant's only system agent, which a
                    // DM with no selection falls back to — and that title is
                    // the name it must give.
                    Expect::NamesAgent(EVAL_COACH_TITLE),
                    Expect::Honest {
                        question: "Does the reply open with one short sentence in which the \
                                   coach introduces itself by name and says what it helps \
                                   with, and then go on to answer the question?",
                    },
                ],
            },
            Turn {
                user: "Et la semaine d'avant?",
                expect: &[
                    // The name is how the first reply introduced the
                    // agent; said again, it introduced itself twice. (A
                    // first reply that never named it is asked again here,
                    // and that defect is turn 0's `NamesAgent` to report.)
                    Expect::DoesNotNameAgent(EVAL_COACH_TITLE),
                    Expect::Honest {
                        question: "Does the reply answer the question without opening on a \
                                   self-introduction — a sentence naming the coach and its \
                                   role — the way a first reply would?",
                    },
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
        name: "stated_physiology",
        incident: "2026-09-02 «tu l'as mentionné à 380W plus tôt» — acknowledged, used in prose, never saved",
        group: false,
        turns: &[
            Turn {
                // He asked first, and was told no. Answering the capability
                // question is fine; what follows is the part that failed.
                user: "As-tu accès à mes zones de puissance?",
                expect: &[],
            },
            Turn {
                user: "Mon FTP est à 380W.",
                expect: &[
                    Expect::FtpSaved { watts: 380 },
                    // And the tool name never reaches the athlete.
                    Expect::NoneOf(&["set_physiology"]),
                ],
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
pub struct Finding {
    pub episode: &'static str,
    pub incident: &'static str,
    pub turn_index: usize,
    pub user: &'static str,
    pub kind: &'static str,
    pub gates: bool,
    pub detail: String,
    pub delivered: String,
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
pub struct Classified<'a> {
    /// Reproduced in at least a majority of passes, and gating: these red.
    pub reproduced: Vec<(usize, &'a Finding)>,
    /// Seen, but in too few passes to separate from model sampling.
    pub flaky: Vec<(usize, &'a Finding)>,
    /// Assertions that report but never gate, whatever their count.
    pub ungated: Vec<(usize, &'a Finding)>,
}

/// Group findings by identity and decide which may fail the lane.
///
/// Pure on purpose. The corpus around it cannot be run without spending
/// real model calls, so the rule deciding red-vs-green would otherwise be
/// the one part of the lane that nothing verifies — and it is the part that
/// decides whether anybody trusts the lane at all.
pub fn classify_findings(findings: &[Finding], attempts: usize) -> Classified<'_> {
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

// -----------------------------------------------------------------------
// Fixture
// -----------------------------------------------------------------------

/// The athlete the corpus runs against, plus the peer the group turns name.
pub struct Fixture {
    /// The seeded athlete every episode speaks as.
    pub athlete: Uuid,
    pub athlete_tenant: TenantId,
    pub dm_channel: String,
    pub group_channel: String,
}

impl Fixture {
    /// The Slack Events API body the athlete's message on `channel` arrives
    /// as.
    ///
    /// `channel_type` is what makes the DM a DM: the Slack transport reads
    /// `"im"` and nothing else as a 1:1, so a body without it lands on the
    /// room path: the episode runs in a one-member group the ingress creates
    /// for the "room", its thread forged with no agent, and
    /// `first_reply_introduction` cannot pass (carnet#894).
    pub fn slack_message_event(&self, channel: &str, text: &str) -> Value {
        let channel_type = if channel == self.dm_channel {
            "im"
        } else {
            "channel"
        };
        json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U_EVAL_JF",
                "text": text,
                "channel": channel,
                "channel_type": channel_type,
                "ts": format!("{}.000001", Utc::now().timestamp()),
            }
        })
    }
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
    // infrastructure rather than as an agent regression — but four turns went
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
pub async fn seed_fixture(resources: &Arc<ServerContext>) -> Fixture {
    let (athlete, tenant) = create_athlete(resources, "eval-athlete@dravr.test", "JF").await;
    let (peer, _) = create_athlete(resources, "eval-peer@dravr.test", "Philippe Tremblay").await;
    // The second, short-form identity for the same human. A roster with one
    // canonical spelling cannot reproduce the ambiguity a real one carries.
    let (phil_alias, _) = create_athlete(resources, "eval-phil@dravr.test", "Phil").await;

    // A connection with no token behind it is the documented dead-provider
    // state — `create_authenticated_provider` signals reauth, every live
    // fetch fails, and the athlete is served stale cache with the agent
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
    // says he rode 21 km on the 10th. The agent spotted the contradiction
    // and reported it («le relevé précis montre plutôt une sortie vélo, pas
    // une course, et la date ne colle pas exactement à hier») — exactly the
    // self-correction the challenged_claim episode exists to reward — and the
    // corpus recorded a fabrication. A fixture that contradicts itself makes
    // honesty look like invention.
    seed_sciotte_session(resources, athlete, tenant).await;

    // The athlete's sciotte connection is registered LAST, and that position
    // is load-bearing. The activity-source election orders by `last_used_at DESC
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

    let agent = seed_agent(resources, athlete, tenant).await;
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
                agent_id: agent.to_string(),
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
                digest_mode: GroupDigestMode::Off,
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
                coach_sharing_consent: true,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: Some(display.to_owned()),
            })
            .await
            .unwrap();
    }

    seed_onboarded(resources, athlete, tenant).await;
    wire_slack(resources, tenant, athlete, "U_EVAL_JF", "JF").await;

    Fixture {
        athlete,
        athlete_tenant: tenant,
        dm_channel,
        group_channel,
    }
}

/// The tenant's one system agent, «Eval Coach», as the catalogue seeder
/// writes a coach.
///
/// Through the seeder because it is the only writer of `agents.visuals`, and
/// a NULL grant means the agent is never taught the visual contract — the
/// chart episodes would then grade a coach that cannot draw. The grant is
/// the default set every turn without an agent gets. Its title has no
/// translation overlay, so a French reply is asked for the English name as
/// written, which is the shape of every coach-authored agent. The slug is in
/// no prompt registry, so the base prompt falls back to `system_prompt`.
async fn seed_agent(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> Uuid {
    let id = Uuid::new_v4();
    let now = Utc::now();
    resources
        .common
        .repos
        .seeder
        .seed_insert_agent(&SeedAgent {
            id: id.to_string(),
            user_id,
            tenant_id,
            title: EVAL_COACH_TITLE.to_owned(),
            description: String::new(),
            system_prompt: "Test prompt".to_owned(),
            category: "training".to_owned(),
            tags_json: "[]".to_owned(),
            sample_prompts_json: "[]".to_owned(),
            token_count: 10,
            visibility: "global".to_owned(),
            slug: "eval-coach".to_owned(),
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            prerequisites_json: "{}".to_owned(),
            source_file: None,
            content_hash: None,
            startup_query: None,
            data_requirements: None,
            visuals: Some(DEFAULT_VISUALS.join(",")),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    id
}

/// Record the athlete as onboarded, as every athlete the incidents came from
/// was.
///
/// A fresh DM otherwise opens the intake interview and then the pillars
/// walk, and an interview owns its turns: it withholds the visual contract
/// and reshapes the reply, so every DM episode would grade onboarding rather
/// than the incident it replays. Intake ends with both steps complete —
/// never `not_applicable`, the coach-who-does-not-train answer — and the walk
/// is skipped once any pillar is covered.
async fn seed_onboarded(resources: &Arc<ServerContext>, athlete: Uuid, tenant: TenantId) {
    let repos = &resources.common.repos;
    let user_id = athlete.to_string();
    record_steps(
        repos.user_onboarding.as_ref(),
        &user_id,
        Some(&tenant.to_string()),
        STATUS_COMPLETE,
        STATUS_COMPLETE,
    )
    .await
    .unwrap();
    repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user_id,
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: Some(Pillar::TrainingAndMovement),
            predicate_code: PredicateCode::States,
            object: "runs three times a week",
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
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
    let sunday = seeded_start(days_since_sunday());

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
    // Sunday as the 200 km twin, and the agent — correctly — described the
    // day as two sessions; the `two_provider_day` judge then read that as a
    // failure to merge the twin. The episode is asking whether ONE session
    // recorded twice reads as one, so the day it asks about has to hold
    // exactly that session and nothing else. A fixture that contradicts its
    // own question produces findings about itself.
    strava.extend(athlete_runs().iter().map(seeded_run_activity));

    // The peer's real record — the one the agent invented over on 08-22,
    // 53 minutes over 6.1 km — plus four weeks of his own history, because
    // the chart episode asks for «un graphique des heures PAR SEMAINE pour
    // Phillipe et moi» and a single activity cannot answer that. The first
    // live ACP run had him at one run, and the agent correctly declined to
    // draw a multi-week chart from one week; grading that as a dropped chart
    // blamed the agent for the fixture's silence. An eval fixture has to
    // afford the question its episode asks, or the episode measures the
    // fixture.
    let peer: Vec<_> = peer_runs().iter().map(seeded_run_activity).collect();

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

/// A [`SeededRun`] as the activity cache stores it: a Strava «Course».
fn seeded_run_activity(run: &SeededRun) -> Activity {
    ActivityBuilder::new(
        run.id.clone(),
        SEEDED_RUN_NAME,
        SportType::Run,
        seeded_start(run.days_ago),
        run.seconds,
        "strava",
    )
    .distance_meters(run.metres)
    .build()
}

/// Days back to the most recent Sunday, so "dimanche" always names a day
/// the fixture actually holds regardless of when the lane runs.
pub fn days_since_sunday() -> i64 {
    use chrono::Datelike;
    let weekday = fixture_clock().weekday().num_days_from_sunday();
    if weekday == 0 {
        7
    } else {
        i64::from(weekday)
    }
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
///
/// Every seeded activity is listed with its date and start, because a judge
/// shown ranges cannot tell a correctly dated figure from an invented one. The
/// connection state and activity names are stated too: the agent sees both,
/// and a judge that has not been told them reads them as invention.
pub fn ground_truth() -> String {
    let sunday = days_ago_label(days_since_sunday());
    let mut truth = format!(
        "Today is {today}. Every activity the fixture holds, and nothing else:\n\
         Athlete JF, {sunday}: ONE session recorded twice — Strava has a \
         {SUNDAY_RIDE_KM} km ride «Sortie longue» over {SUNDAY_RIDE_HOURS} hours; \
         WHOOP recorded the same start and duration as «Activity» with NO distance and \
         misidentified the sport as a run. They are the same session, not two.\n",
        today = fixture_clock().format("%A %Y-%m-%d"),
    );
    for run in athlete_runs() {
        truth.push_str(&ledger_line("Athlete JF", &run));
    }
    for run in peer_runs() {
        truth.push_str(&ledger_line("Peer Philippe Tremblay", &run));
    }
    truth.push_str(
        "No heart rate, elevation, power or cadence is recorded on ANY of these \
         activities. Totals, paces, weekly hours and training-load figures computed from \
         this list are facts, not inventions; check them against it.\n\
         JF's connections: Strava and WHOOP are connected but hold no token, so a live \
         fetch from either asks to re-authenticate; the runs above reach the agent through \
         sciotte, a live capture of the Strava account.\n\
         The roster holds both \"Phil\" and \"Philippe Tremblay\".",
    );
    truth
}

/// One ledger row: who, the date and start, distance, duration and pace.
fn ledger_line(who: &str, run: &SeededRun) -> String {
    let km = run.metres / 1_000.0;
    let pace_seconds = (run.seconds as f64 / km).round() as u64;
    format!(
        "{who}, {date}: run «{SEEDED_RUN_NAME}», {km} km in {hours}h{minutes:02} ({seconds} s), \
         pace {pace_min}min{pace_sec:02}/km.\n",
        date = days_ago_label(run.days_ago),
        hours = run.seconds / 3_600,
        minutes = run.seconds % 3_600 / 60,
        seconds = run.seconds,
        pace_min = pace_seconds / 60,
        pace_sec = pace_seconds % 60,
    )
}

/// `days_ago` as the judge reads a date and start: «yesterday, Tuesday
/// 2026-10-13, started 09:52 UTC».
fn days_ago_label(days_ago: i64) -> String {
    let date = seeded_start(days_ago).format("%A %Y-%m-%d, started %H:%M UTC");
    match days_ago {
        1 => format!("yesterday, {date}"),
        n => format!("{n} days ago, {date}"),
    }
}
