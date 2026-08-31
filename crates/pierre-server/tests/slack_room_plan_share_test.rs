// ABOUTME: Slack-channel e2e for /plan vs /plan share in a shared room — driven through the real
// ABOUTME: Slack webhook ingress with genuinely non-DM channel events, plus the real adapter seams

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#132 landed the shared-room mechanics for `/plan` and `/plan share`,
//! e2e-tested on a TELEGRAM supergroup only. Slack's halves are different in
//! kind, and each is pinned here:
//!
//! - the group signal is `event.channel_type` — canot reads only `"im"` as a
//!   1:1 for message events, so the events below carry `"channel"` explicitly
//!   and the ingress must treat them as a shared room;
//! - Slack has no message-deletion API (`SlackChannel` keeps the trait
//!   default), so a slash-command echo always survives and the room is always
//!   owed the answered-privately notice;
//! - the private mechanism is an in-channel ephemeral (`chat.postEphemeral`
//!   with a `user` field), never a DM redirect;
//! - threading is `thread_ts` on the rendered payload, mapped from
//!   [`OutgoingMessage::reply_to`].
//!
//! The slash egress writes the delivery ledger (carnet#158): the athlete's
//! `/…` line lands as an inbound `messaging_messages` row synchronously, and
//! each spawned reply part lands as an outbound row after its send resolves.
//! Through the real webhook the Slack API call fails (test credentials), so
//! the observable outbound rows are the `failed-…`-keyed attempts — which is
//! the point: the wire stays observable from the database either way. The
//! ledger is an operator surface with no member-readable reader (only the
//! emoji-reaction resolver joins through it), so a durable operator copy of a
//! privately-answered `/plan` is deliberate; the member-readable surfaces
//! (chat rows, group transcript) keep their deliberate absences, asserted
//! below. The delivery halves are still asserted against the REAL canot
//! `SlackChannel` at the public seams the ingress calls: `settle_room_echo`,
//! `ephemeral_payload`, and `render`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod slack_room {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use pierre_chat_pipeline::stages::command_persistence::is_room_visible;
    use pierre_contremaitre::messaging_strings::{
        KEY_PLAN_SHARED_HEADER, KEY_SLASH_ANSWERED_PRIVATELY,
    };
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupMember, GroupRespondMode, GroupRole, TranscriptSpeaker,
    };
    use pierre_core::models::messaging::{ChannelType, MessageContent, OutgoingMessage};
    use pierre_core::models::{Tenant, TenantId, User, UserStatus, COMMAND_FINISH_REASON};
    use pierre_database::backends::factory::Database;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::messaging_ingress::room_echo::{
        settle_room_echo, RoomEchoSettlement,
    };
    use pierre_memory::training_plans::{
        BlockPhase, GoalRace, PlanBlock, PlannedDay, RacePriority,
    };
    use pierre_messaging::channel::MessagingChannel;
    use pierre_messaging::channels::slack::{ephemeral_payload, SlackChannel};
    use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
    use serde_json::{json, Value};
    use sha2::Sha256;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    const SIGNING_SECRET: &str = "slack_room_plan_secret";
    /// The shared Slack channel every scenario binds its group to.
    const ROOM: &str = "C_SLACK_PLAN_ROOM";
    const SHARER_SENDER: &str = "U_SLACK_SHARER";
    const PEER_SENDER: &str = "U_SLACK_PEER";
    /// The roster name the shared header must carry.
    const SHARER_NAME: &str = "Sacha Lévesque";
    /// A session text unique to the seeded plan week — the marker that the
    /// plan reached (or must not reach) a persisted surface.
    const PLAN_SESSION: &str = "endurance ride";

    /// Compute the Slack webhook signature (`v0=<hex-hmac-sha256>` over
    /// `v0:{ts}:{body}`).
    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// An active user with a display name and their own tenant.
    async fn seed_named_user(
        resources: &Arc<ServerContext>,
        email: &str,
        display_name: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some(display_name.to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Slack Room Tenant {email}"),
            slug: format!("slack-room-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
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

    /// A coach persona under `tenant`. The selected-coach pointer is a foreign
    /// key onto `coaches`, so a plan's coach slug has to be a real persona id.
    async fn seed_persona(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant: TenantId,
        title: &str,
    ) -> String {
        resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                tenant,
                &CreateSystemCoachRequest {
                    title: title.to_owned(),
                    description: None,
                    system_prompt: "You are a concise test coach.".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap()
            .id
            .to_string()
    }

    /// One outline block plus one stored week straddling today, filed under
    /// the athlete's selected coach — the same shape
    /// `plan_command_test::seed_plan_with` builds, so every `/plan` view has a
    /// real session to show.
    async fn seed_week_plan(
        resources: &Arc<ServerContext>,
        user: Uuid,
        tenant: TenantId,
        coach_slug: &str,
    ) {
        let today = Utc::now().date_naive();
        let start = today - chrono::Days::new(2);
        let date = |offset: u64| {
            (start + chrono::Days::new(offset))
                .format("%Y-%m-%d")
                .to_string()
        };
        let day = |offset: u64, sport: &str, workout: &str, minutes: Option<u32>| PlannedDay {
            date: date(offset),
            sport: sport.to_owned(),
            workout: workout.to_owned(),
            duration_min: minutes,
            intensity: "Z2".to_owned(),
            steps: Vec::new(),
            fueling: None,
        };
        let goal = GoalRace {
            name: "Unbound XL".to_owned(),
            date: "2026-10-03".to_owned(),
            discipline: "gravel".to_owned(),
            priority: RacePriority::A,
        };
        let blocks = vec![PlanBlock {
            phase: BlockPhase::Build,
            start: date(0),
            weeks: 4,
            intent: "volume up".to_owned(),
            target_hours: Some(14.0),
        }];
        // Offset 2 is today, offset 3 tomorrow — the compact, week and today
        // views all land on PLAN_SESSION.
        let days = vec![
            day(0, "rest", "full rest", None),
            day(1, "gravel", "VO2 intervals", Some(90)),
            day(2, "gravel", PLAN_SESSION, Some(120)),
            day(3, "run", "easy shakeout", Some(40)),
        ];
        let week_start = date(0);
        resources
            .common
            .repos
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &tenant.to_string(),
                user_id: &user.to_string(),
                coach_slug: Some(coach_slug),
                goal_fact_id: None,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: &[],
                    strategy: "rebuild volume then sharpen",
                    blocks: &blocks,
                    source_conversation_id: None,
                }),
                weeks: &[PlanWeekInput {
                    week_start: &week_start,
                    focus: "build volume",
                    days: &days,
                    adjustment_reason: "",
                }],
            })
            .await
            .unwrap();
    }

    async fn link(
        resources: &Arc<ServerContext>,
        bot_tenant: TenantId,
        user_id: Uuid,
        sender: &str,
        label: &str,
    ) {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            user_id: &user_id.to_string(),
            channel_type: "slack",
            channel_user_id: sender,
            display_name: Some(label),
        })
        .await
        .unwrap();
    }

    async fn add_member(
        resources: &Arc<ServerContext>,
        group_id: Uuid,
        user_id: Uuid,
        bot_tenant: TenantId,
        role: GroupRole,
    ) {
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: bot_tenant.to_string(),
                role,
                // Both members consent, so the room transcript is readable
                // room-wide and the share fan-out is asserted through the
                // peer's eyes, not only its author's.
                peer_sharing_consent: true,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// The sharer (a plan under their selected coach in their own tenant) and
    /// a peer, both linked to one Slack CHANNEL bound to a coaching group
    /// under the bot tenant.
    struct SlackRoom {
        resources: Arc<ServerContext>,
        bot_tenant: TenantId,
        group_id: Uuid,
        sharer: Uuid,
        sharer_tenant: TenantId,
        peer: Uuid,
    }

    async fn build_slack_room() -> SlackRoom {
        let resources = create_test_server_resources().await.unwrap();

        let (sharer, sharer_tenant) = seed_named_user(
            &resources,
            &format!("slack-sharer-{}@example.com", Uuid::new_v4()),
            SHARER_NAME,
        )
        .await;
        // The plan their DM built: filed under the coach they selected in
        // their own tenant — the room read must resolve it through the
        // selected-coach rung, since the room conversation binds no coach the
        // athlete's tenant knows.
        let selected = seed_persona(&resources, sharer, sharer_tenant, "Share Coach").await;
        resources
            .common
            .repos
            .tenants
            .set_selected_coach(sharer_tenant, sharer, Some(&selected))
            .await
            .unwrap();
        seed_week_plan(&resources, sharer, sharer_tenant, &selected).await;

        let (peer, _peer_tenant) = seed_named_user(
            &resources,
            &format!("slack-peer-{}@example.com", Uuid::new_v4()),
            "Paulo Costa",
        )
        .await;
        let (bot_owner, bot_tenant) = seed_named_user(
            &resources,
            &format!("slack-bot-{}@example.com", Uuid::new_v4()),
            "Bot Owner",
        )
        .await;

        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            channel_type: "slack",
            api_key: Some("xoxb-slack-room-plan"),
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
        link(&resources, bot_tenant, sharer, SHARER_SENDER, "Sacha").await;
        link(&resources, bot_tenant, peer, PEER_SENDER, "Paulo").await;

        // The group bound to the Slack channel, under the BOT tenant — the
        // row `resolve_or_create_channel_group` finds instead of
        // bootstrapping one, exactly as the Telegram room fixtures pre-bind.
        let room_persona = seed_persona(&resources, bot_owner, bot_tenant, "Room Coach").await;
        let group_id = Uuid::new_v4();
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .create_group(
                bot_tenant,
                &CoachingGroup {
                    id: group_id,
                    tenant_id: bot_tenant.to_string(),
                    name: "Slack Plan Room".to_owned(),
                    description: None,
                    coach_id: room_persona,
                    owner_id: sharer,
                    coach_user_id: None,
                    peer_data_sharing: true,
                    respond_mode: GroupRespondMode::default(),
                    max_members: 10,
                    is_active: true,
                    channel_type: Some("slack".to_owned()),
                    channel_chat_id: Some(ROOM.to_owned()),
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        add_member(&resources, group_id, sharer, bot_tenant, GroupRole::Owner).await;
        add_member(&resources, group_id, peer, bot_tenant, GroupRole::Member).await;

        SlackRoom {
            resources,
            bot_tenant,
            group_id,
            sharer,
            sharer_tenant,
            peer,
        }
    }

    /// POST one Slack CHANNEL message through the real webhook ingress.
    ///
    /// The event carries `channel_type: "channel"` because that field is what
    /// canot's transport keys on for message events (`"im"` is the only DM);
    /// the earlier Slack e2e events omitted it, which already reads as
    /// non-DM, but a group fixture must state its group signal rather than
    /// rely on an absence.
    async fn post_channel_message(room: &SlackRoom, sender: &str, ts: &str, text: &str) {
        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": sender,
                "text": text,
                "channel": ROOM,
                "channel_type": "channel",
                "ts": ts
            }
        })
        .to_string();
        let timestamp = Utc::now().timestamp().to_string();
        let sig = compute_slack_sig(SIGNING_SECRET, &timestamp, &body);
        let router = MessagingRoutes::routes(Arc::clone(&room.resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &sig)
            .text(&body)
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    /// The command-stamped rows of `user`'s conversations under `tenant`, as
    /// `(role, content)` pairs. Slash handling is synchronous inside the
    /// webhook call, so no polling: whatever was going to persist has.
    async fn command_rows_under(
        resources: &Arc<ServerContext>,
        tenant: TenantId,
        user: Uuid,
    ) -> Vec<(String, String)> {
        const SQL: &str = "SELECT m.role, m.content FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = $1 AND CAST(c.user_id AS TEXT) = $2 \
               AND m.finish_reason = $3";
        match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_as(SQL)
                .bind(tenant.to_string())
                .bind(user.to_string())
                .bind(COMMAND_FINISH_REASON)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_as(SQL)
                .bind(tenant.to_string())
                .bind(user.to_string())
                .bind(COMMAND_FINISH_REASON)
                .fetch_all(db.pool())
                .await
                .unwrap(),
        }
    }

    /// Rows in `chat_messages` whose content carries `needle`, across every
    /// tenant — the "did the plan text land anywhere durable" probe.
    async fn chat_rows_carrying(resources: &Arc<ServerContext>, needle: &str) -> i64 {
        const SQL: &str = "SELECT COUNT(*) FROM chat_messages WHERE content LIKE '%' || $1 || '%'";
        match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(needle)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(needle)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        }
    }

    /// The ledger rows carrying `needle`:
    /// `(direction, channel_message_id, chat_message_id, correlation_id, tenant_id)`.
    async fn ledger_rows_carrying(
        resources: &Arc<ServerContext>,
        needle: &str,
    ) -> Vec<(String, String, Option<String>, String, String)> {
        const SQL: &str = "SELECT direction, channel_message_id, chat_message_id,                                   correlation_id, tenant_id              FROM messaging_messages WHERE content_body LIKE '%' || $1 || '%'              ORDER BY created_at ASC";
        match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_as(SQL)
                .bind(needle)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_as(SQL)
                .bind(needle)
                .fetch_all(db.pool())
                .await
                .unwrap(),
        }
    }

    /// The reply parts are persisted by the spawned delivery task after the
    /// real Slack call resolves (here: fails on test credentials), so the
    /// outbound rows appear asynchronously — poll up to ~30s.
    async fn wait_for_outbound_ledger_rows(
        resources: &Arc<ServerContext>,
        needle: &str,
        want: usize,
    ) -> Vec<(String, String, Option<String>, String, String)> {
        for _ in 0..300 {
            let rows: Vec<_> = ledger_rows_carrying(resources, needle)
                .await
                .into_iter()
                .filter(|r| r.0 == "outbound")
                .collect();
            if rows.len() >= want {
                return rows;
            }
            sleep(Duration::from_millis(100)).await;
        }
        panic!("no {want} outbound ledger row(s) carrying {needle:?} appeared");
    }

    /// The id of the command-stamped assistant chat row carrying the shared
    /// plan — what the outbound ledger row must stamp as `chat_message_id`.
    async fn assistant_chat_row_id(resources: &Arc<ServerContext>) -> String {
        const SQL: &str = "SELECT id FROM chat_messages              WHERE finish_reason = $1 AND role = 'assistant'                AND content LIKE '%' || $2 || '%'";
        match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(COMMAND_FINISH_REASON)
                .bind(PLAN_SESSION)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(COMMAND_FINISH_REASON)
                .bind(PLAN_SESSION)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        }
    }

    /// Bare `/plan` typed in a Slack channel is a private matter: nothing of
    /// the plan may reach any room-readable surface, and the room — whose
    /// echo Slack cannot delete — is owed the one line saying where the
    /// answer went.
    #[tokio::test]
    async fn plan_in_a_slack_channel_is_answered_privately_with_a_room_notice() {
        let room = build_slack_room().await;
        post_channel_message(&room, SHARER_SENDER, "1700000100.000001", "/plan").await;

        // A private command in a shared room writes no CHAT rows: had the
        // event been read as a DM, `CommandPersistence::Always` would have
        // written the `/plan` pair — so these absences also pin the group
        // signal.
        assert_eq!(
            chat_rows_carrying(&room.resources, "/plan").await,
            0,
            "a private command's turn must not be written in a shared room"
        );
        assert_eq!(
            chat_rows_carrying(&room.resources, PLAN_SESSION).await,
            0,
            "no member-readable copy of the plan may exist for a private room answer"
        );
        // The DELIVERY LEDGER holds the turn (carnet#158): the `/…` line as an
        // inbound row (written synchronously in the webhook call, under the
        // room's session tenant), and the private reply's parts as outbound
        // rows once the spawned send resolves. `messaging_messages` has no
        // member-readable surface — only the emoji-reaction resolver joins
        // through it, and a private reply carries no chat row to resolve to —
        // so the operator copy is deliberate, not a leak of the room policy.
        let inbound: Vec<_> = ledger_rows_carrying(&room.resources, "/plan")
            .await
            .into_iter()
            .filter(|r| r.0 == "inbound")
            .collect();
        assert_eq!(
            inbound.len(),
            1,
            "the command line is one inbound ledger row"
        );
        assert_eq!(
            inbound[0].4,
            room.bot_tenant.to_string(),
            "a room turn's ledger rows file under the room's session tenant"
        );
        let outbound = wait_for_outbound_ledger_rows(&room.resources, PLAN_SESSION, 1).await;
        for row in &outbound {
            assert!(
                row.1.starts_with("failed-"),
                "the test-credential Slack send fails, and the attempt is the record: {row:?}"
            );
            assert!(
                row.2.is_none(),
                "a private reply is never chat-persisted, so there is nothing to rate: {row:?}"
            );
            assert_eq!(
                row.3, inbound[0].3,
                "reply and command share one correlation id (the inbound turn)"
            );
        }
        let entries = room
            .resources
            .common
            .repos
            .groups
            .list_transcript_visible_to(&room.group_id.to_string(), room.sharer, 10)
            .await
            .unwrap();
        assert!(
            entries.is_empty(),
            "a privately answered command never reaches the room transcript: {entries:?}"
        );

        // The room's half, at the seam the ingress calls, with the REAL Slack
        // adapter: Slack has no message-deletion API (`SlackChannel` keeps
        // the trait default), so the echo survives every time and the
        // settlement must produce the room-addressed answered-privately
        // notice.
        let adapter: Arc<dyn MessagingChannel> =
            Arc::new(SlackChannel::new(SIGNING_SECRET.to_owned()));
        let sharer_id = room.sharer.to_string();
        let notice = settle_room_echo(RoomEchoSettlement {
            resources: &room.resources,
            db: &*room.resources.common.repos.messaging,
            tenant_id: room.bot_tenant,
            channel: "slack",
            channel_type: ChannelType::Slack,
            adapter: &adapter,
            room_id: ROOM,
            channel_message_id: "1700000100.000001",
            user_id: &sharer_id,
            sender_id: SHARER_SENDER,
        })
        .await
        .expect("Slack cannot delete the echo, so the room must be told where the answer went");
        assert_eq!(
            notice.recipient_id, ROOM,
            "the notice belongs to the channel, not to the caller's private surface"
        );
        let MessageContent::Text { body } = &notice.content else {
            panic!("expected a plain-text notice, got {:?}", notice.content);
        };
        let expected = room
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_SLASH_ANSWERED_PRIVATELY, "fr");
        assert_eq!(
            body, &expected,
            "the notice is the catalogued answered-privately line in the caller's locale"
        );
        assert!(
            !body.contains(PLAN_SESSION),
            "the notice carries no part of the plan: {body}"
        );

        // The caller's half: canot's Slack private mechanism is an in-channel
        // ephemeral — `send_private_reply` renders the room payload, adds the
        // caller as `user`, and posts it to chat.postEphemeral. The payload
        // shape is the whole contract: targeted at the caller, inside the
        // channel the command came from, never a DM redirect.
        let reply = OutgoingMessage {
            channel_type: ChannelType::Slack,
            recipient_id: ROOM.to_owned(),
            content: MessageContent::RichText {
                body: format!("Today: {PLAN_SESSION}"),
            },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        };
        let payload = ephemeral_payload(adapter.render(&reply).unwrap(), SHARER_SENDER);
        assert_eq!(
            payload["user"],
            Value::from(SHARER_SENDER),
            "the ephemeral targets the caller alone"
        );
        assert_eq!(
            payload["channel"],
            Value::from(ROOM),
            "the ephemeral shows inside the channel the command came from"
        );
    }

    /// `/plan share week` posts the plan to the channel: the reply opens with
    /// the shared header naming the athlete, the turn's pair of rows lands in
    /// the room conversation under the BOT tenant (not the athlete's own),
    /// and the turn fans out to the group transcript — the member's line and
    /// the coach's answer, readable by the peer, not only its author.
    #[tokio::test]
    async fn plan_share_in_a_slack_channel_is_posted_to_the_channel_with_the_header() {
        let room = build_slack_room().await;
        post_channel_message(
            &room,
            SHARER_SENDER,
            "1700000200.000001",
            "/plan share week",
        )
        .await;

        let rows = command_rows_under(&room.resources, room.bot_tenant, room.sharer).await;
        let user_row = rows
            .iter()
            .find(|(role, _)| role == "user")
            .unwrap_or_else(|| panic!("the member's command line must persist, got {rows:?}"));
        assert_eq!(user_row.1, "/plan share week");

        let header = room.resources.mcp.messaging_strings_registry.render(
            KEY_PLAN_SHARED_HEADER,
            "fr",
            &[SHARER_NAME],
        );
        let assistant = rows
            .iter()
            .find(|(role, _)| role == "assistant")
            .unwrap_or_else(|| panic!("the room-visible reply must persist, got {rows:?}"));
        assert!(
            assistant.1.contains(&header),
            "the reply opens with the shared header naming the athlete \
             (expected {header:?} in {:?})",
            assistant.1
        );
        assert!(
            assistant.1.contains(PLAN_SESSION),
            "the reply carries the seeded week: {:?}",
            assistant.1
        );

        // The rows live under the BOT tenant: the event was read as a channel
        // message. A DM-read event files the pair under the sharer's own
        // tenant instead.
        assert!(
            command_rows_under(&room.resources, room.sharer_tenant, room.sharer)
                .await
                .is_empty(),
            "a room turn must not be filed under the athlete's own tenant"
        );

        // The room-visible reply lands in the delivery ledger (carnet#158):
        // outbound rows carrying the shared plan, stamped with the assistant
        // chat row so an emoji reaction on the delivered message resolves to
        // a message to rate. Through the real webhook the Slack send fails on
        // test credentials, so the rows are `failed-…`-keyed attempts — the
        // stamp and the correlation are what this pins.
        let assistant_chat_id = assistant_chat_row_id(&room.resources).await;
        let outbound = wait_for_outbound_ledger_rows(&room.resources, PLAN_SESSION, 1).await;
        for row in &outbound {
            assert_eq!(
                row.2.as_deref(),
                Some(assistant_chat_id.as_str()),
                "the outbound row stamps the assistant chat row for reaction rating: {row:?}"
            );
        }
        let inbound: Vec<_> = ledger_rows_carrying(&room.resources, "/plan share week")
            .await
            .into_iter()
            .filter(|r| r.0 == "inbound")
            .collect();
        assert_eq!(
            inbound.len(),
            1,
            "the command line is one inbound ledger row"
        );
        assert_eq!(
            inbound[0].3, outbound[0].3,
            "both ledger directions share the inbound turn's correlation id"
        );

        // The turn fanned out to the shared transcript — the member line and
        // the coach answer, both attributed to the sharer — and the PEER can
        // read both (group sharing on, sharer consenting).
        for viewer in [room.sharer, room.peer] {
            let entries = room
                .resources
                .common
                .repos
                .groups
                .list_transcript_visible_to(&room.group_id.to_string(), viewer, 10)
                .await
                .unwrap();
            assert_eq!(
                entries.len(),
                2,
                "viewer {viewer} must see the member line and the coach answer: {entries:?}"
            );
            let member = entries
                .iter()
                .find(|e| matches!(e.speaker, TranscriptSpeaker::Member))
                .unwrap_or_else(|| panic!("no member entry for viewer {viewer}: {entries:?}"));
            assert_eq!(member.content, "/plan share week");
            assert_eq!(member.author_user_id, room.sharer);
            let coach = entries
                .iter()
                .find(|e| matches!(e.speaker, TranscriptSpeaker::Coach))
                .unwrap_or_else(|| panic!("no coach entry for viewer {viewer}: {entries:?}"));
            assert!(
                coach.content.contains(&header) && coach.content.contains(PLAN_SESSION),
                "the fanned-out reply is the room-visible one, header and week included: {:?}",
                coach.content
            );
            assert_eq!(
                coach.author_user_id, room.sharer,
                "the coach's answer is attributed to the member it answered"
            );
        }
    }

    /// The room-visible reply threads onto the command echo. The DELIVERY
    /// outcome is ledgered now, but `reply_to` has no ledger column, so the
    /// two halves the platform owns are pinned at the [`OutgoingMessage`]
    /// level: `plan-share` routes through the
    /// room-visible branch — the one arm of `dispatch_slash_command_if_any`
    /// that sets `reply_to` to the inbound channel message id — and the REAL
    /// Slack adapter maps `reply_to` to Slack's threading field, `thread_ts`,
    /// on the payload addressed to the channel. (Confirmed live: the shared
    /// reply arrives threaded under the command message, not top-level.)
    #[test]
    fn plan_share_reply_in_a_slack_channel_threads_to_the_command() {
        // The routing predicate: a room-visible command's reply is posted
        // back to the room and threads onto the echo, while bare `/plan` is
        // redirected privately and threads nothing.
        assert!(
            is_room_visible(Some("plan-share")),
            "plan-share must take the room-visible branch that sets reply_to"
        );
        assert!(
            !is_room_visible(Some("plan")),
            "bare /plan must keep the private default and never thread into the room"
        );

        let inbound_ts = "1700000300.000042";
        let slack = SlackChannel::new(SIGNING_SECRET.to_owned());
        let reply = OutgoingMessage {
            channel_type: ChannelType::Slack,
            recipient_id: ROOM.to_owned(),
            content: MessageContent::RichText {
                body: format!("<b>{SHARER_NAME}</b> — {PLAN_SESSION}"),
            },
            turn_id: CanotTurnId::new(),
            reply_to: Some(inbound_ts.to_owned()),
            thread_id: None,
        };
        let payload = slack.render(&reply).unwrap();
        assert_eq!(
            payload["thread_ts"],
            Value::from(inbound_ts),
            "reply_to must reach Slack as thread_ts — the only linkage Slack threads on"
        );
        assert_eq!(
            payload["channel"],
            Value::from(ROOM),
            "the threaded reply stays addressed to the channel the command came from"
        );
        // The RichText path renders the header's bold as mrkdwn, so the
        // threaded payload carries `*…*` rather than raw HTML (confirmed
        // live: the header shows as bold in the thread).
        let block_text = payload["blocks"][0]["text"]["text"]
            .as_str()
            .expect("rendered section block carries text");
        assert!(
            block_text.contains(&format!("*{SHARER_NAME}*")),
            "the <b> header must render as Slack mrkdwn bold: {block_text}"
        );

        // Without the linkage no phantom thread field appears.
        let unthreaded = OutgoingMessage {
            reply_to: None,
            ..reply
        };
        assert!(
            slack
                .render(&unthreaded)
                .unwrap()
                .get("thread_ts")
                .is_none(),
            "an unthreaded reply must not invent a thread_ts"
        );
    }
}
