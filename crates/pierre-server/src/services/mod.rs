// ABOUTME: pierre-server-local services that have not been extracted yet
// ABOUTME: Most domain services live in pierre-services, pierre-commands, pierre-chat-pipeline, etc.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! pierre-server-local services
//!
//! Wave 1 leaf services live in the `pierre-services` workspace crate.
//! Chat pipeline lives in `pierre-chat-pipeline`. Slash command parsing
//! lives in `pierre-commands`. Tool execution + group fitness live in
//! `pierre-tool-runtime`. The modules here are the ones that haven't
//! been extracted because they still touch pierre-server-internal
//! resources (Discord/Slack gateways, messaging ingress, endurance
//! training history compute, chat verdict materialization).

/// Importing an athlete's own export through dravr-barrage: stores, link custody, steps, sweeps.
pub mod archive_import;
/// Work this service delivers to itself through Cloud Tasks: task creation and delivery tokens.
pub mod cloud_tasks;
/// One lock per conversation that every chat turn holds while it runs.
pub mod conversation_turn_lock;
/// Tracks the detached turns a webhook starts, so shutdown can drain them.
pub mod turn_lifecycle;
/// Where a messaging turn is started: in this process, or as a Cloud Tasks request.
pub mod turn_runner;

/// Chat verdict service: maps ClaimVerdict rows into chat-facing wire shapes
pub mod chat_verdicts;
/// Client for the photograveur press service (Scene -> PNG).
pub mod photograveur_client;

/// User-facing memory fact service: list and forget what the agent remembers (Sprint C5)
pub mod memory_facts;

/// User-facing persona cards handler: « Style de coaching » from the live contract registry
pub mod personas;

/// One cached activity's splits and laps for its view, read at most once per activity
pub mod activity_detail;

/// One cached activity's drawable route for the Home page, read at most once per activity
pub mod activity_route;

/// A completed workout's `.fit` file uploaded by the athlete: decoded, deduplicated, kept
pub mod activity_upload;

/// The deployment's blob store, which keeps every original file, and the drain of uploads into it
pub mod originals;

/// The periodic backfill that stores the samples of activities uploaded before the sample store held uploads
pub mod upload_streams_backfill;

/// The one-time recompute of every athlete's training history after the workout precedence rule changed
pub mod precedence_recompute;

/// What one Terra webhook event carried, stored through the activity cache and the wellness store
#[cfg(feature = "provider-terra")]
pub mod terra_ingest;

/// The athlete's training status for Home: form and its band, the form trend, load against baseline
pub mod training_status;

/// The athlete's weekly training volume for Home: distance, time and climbing per sport, week by week
pub mod training_volume;

/// The athlete's gear ledger over their cached workouts: lazy default pairs, choices once per workout
pub mod gear_ledger;

/// Discord Gateway WebSocket client — bridges real-time messages and reactions to the webhook pipeline
#[cfg(feature = "client-messaging")]
pub mod discord_gateway;

/// Slack Socket Mode WebSocket client — bridges real-time Slack events to the webhook pipeline
#[cfg(feature = "client-messaging")]
pub mod slack_socket;

/// Messaging ingress: OTP flow, channel linking, session resolution, slash command dispatch
#[cfg(feature = "client-messaging")]
pub mod messaging_ingress;

/// `feedback.recorded`: the one notify event thumbs, reactions and the feedback thread emit (carnet#1235).
#[cfg(any(feature = "client-chat", feature = "client-messaging"))]
pub(crate) mod feedback_events;

/// Account-approved notifier: email + localized message on each linked channel
#[cfg(feature = "client-messaging")]
pub(crate) mod user_approval_notifier;

/// Resolves a tenant's messaging channel to an outbound adapter and its config —
/// the seam every proactive sender shares.
#[cfg(feature = "client-messaging")]
pub mod adapter_resolver;

/// Backfill-completion notifier: pushes a "your history is ready" notice back to
/// the channel that triggered a historical activity backfill.
#[cfg(feature = "client-messaging")]
pub mod backfill_notifier;

/// A verified coach post the platform makes on its own initiative.
#[cfg(feature = "client-messaging")]
pub mod platform_post;

/// Where a post the coach makes on its own lands: the athlete's own thread,
/// and the messaging DM they last wrote from while its window is open.
#[cfg(feature = "client-messaging")]
pub(crate) mod post_delivery;

/// The coach's first post once an athlete's data lands (carnet#831).
#[cfg(all(feature = "client-messaging", feature = "health-sync"))]
pub mod first_sync_post;

/// The coach's check-in on the cadence the athlete chose (carnet#831).
#[cfg(feature = "client-messaging")]
pub mod check_in_post;

/// The two ways a proactive notice reaches an athlete: out through a
/// messaging channel's adapter, or into a first-party conversation as a
/// persisted turn. Gated with the notifier and the first post it serves: the
/// channel arm needs the messaging adapters.
#[cfg(feature = "client-messaging")]
pub(crate) mod backfill_delivery;

/// The activity window a backfill-completion notice lists, read under each
/// provider's terms as a model may see it.
#[cfg(feature = "client-messaging")]
pub(crate) mod backfill_window;

/// Spawns the coaching background workers (outcome evaluator, archetype
/// aggregation, commitment sweep).
pub mod coaching_workers;

/// Turns a commitment sweep's refresh request into background provider
/// fetches, so a quiet athlete's window still gets counted.
pub mod commitment_refresher;

/// Assembles and starts the Strava seat reclaimer (carnet#505).
pub mod seat_reclaim_worker;

/// The periodic walk of each Strava athlete's history that seeds their
/// all-time best efforts before any is announced (carnet#582).
#[cfg(feature = "health-sync")]
pub mod personal_best_seed;

/// Delivers a swept commitment verdict back to the athlete, applying the
/// per-channel proactive-messaging policy.
#[cfg(feature = "client-messaging")]
pub mod commitment_reporter;

/// Posts a group's weekly digest into the group's bound chat — the
/// implementation of the digest scheduler's chat seam.
#[cfg(all(feature = "client-messaging", feature = "client-groups"))]
pub mod group_chat_poster;

/// AuthService-backed credential refresher installed on the health-sync
/// storage post-Arc (the refresh path needs the composition-root runtime).
#[cfg(feature = "health-sync")]
pub mod health_sync_refresher;
