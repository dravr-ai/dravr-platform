// ABOUTME: The group weekly digest goes where the group's mode says — its chat, its managers or nowhere — once a week
// ABOUTME: Drives the scheduler tick with a recording chat poster and channel sink; covers the modes, ledger, locale and poster
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(all(
    feature = "client-notifications",
    feature = "client-messaging",
    feature = "client-groups"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;

mod group_digest_room_tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{DateTime, Datelike, Days, Duration, NaiveDate, TimeZone, Utc};
    use chrono_tz::America::Toronto;
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
        MemberFitnessSnapshot, OvertrainingRiskLevel,
    };
    use pierre_core::models::messaging::{ChannelType, MessageContent};
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::factory::Database;
    use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
    use pierre_database::RepositoryRegistry;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::services::group_chat_poster::ServerGroupChatPoster;
    use pierre_messaging::channels::discord::renderer::DiscordRenderer;
    use pierre_messaging::channels::slack::renderer::SlackRenderer;
    use pierre_messaging::renderer::ResponseRenderer;
    use pierre_notifications::NotificationEvent;
    use pierre_notifications::{
        DispatchRequest, NotificationChannelSink, NotificationService, TenantId as CommTenantId,
    };
    use pierre_routes_groups::group_digest_scheduler::{
        room_digest_params, tick, DigestTickOutcome,
    };
    use pierre_routes_groups::group_digest_slot::week_key;
    use pierre_routes_groups::GroupChatPoster;
    use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
    use pierre_services::notification_text::NotificationTextRenderer;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{Map, Value};
    use uuid::Uuid;

    use crate::common::create_test_server_resources;
    use crate::messaging_fixtures::{
        create_test_db, seed_user, CapturingChannel, FailingChannel, FakeResolver,
    };

    /// The French scope line for a group where `shared` of `roster` share.
    fn french_scope(shared: usize, roster: usize) -> String {
        format!(
            "Ce récap ne couvre que les membres qui partagent leur entraînement avec le groupe \
             ({shared} sur {roster}). Pour y apparaître, envoie /group consent yes"
        )
    }

    const FRENCH_ALL_CLEAR: &str = "Rien à signaler côté forme et assiduité cette semaine.";

    /// The first Monday after today. A week is owed only when its Monday slot
    /// comes after the group was created, and every group here is created at
    /// the real clock, so the ticks are anchored on the Monday still ahead.
    fn next_monday() -> NaiveDate {
        let today = Utc::now().date_naive();
        today + Days::new(7 - u64::from(today.weekday().num_days_from_monday()))
    }

    /// 08:00 in Montreal on [`next_monday`], the slot every zoned group here
    /// is anchored on.
    fn monday_morning_montreal() -> DateTime<Utc> {
        let local = next_monday().and_hms_opt(8, 0, 0).unwrap();
        Toronto
            .from_local_datetime(&local)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    /// `hour:minute` UTC on [`next_monday`], for the no-zone rule.
    fn next_monday_utc(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.from_utc_datetime(&next_monday().and_hms_opt(hour, minute, 0).unwrap())
    }

    // ── Test doubles ────────────────────────────────────────────────────
    //
    // Mocks are test-only: the assertion point is what the scheduler decides to
    // post and to whom. The real poster's escape/split/send path is covered
    // below against a capturing adapter, and the real sink by the messaging
    // sink's own suite.

    /// Records every chat post instead of reaching a channel.
    #[derive(Default)]
    struct RecordingPoster {
        posts: Mutex<Vec<(TenantId, ChannelType, String, String)>>,
        /// When set, the chat takes nothing: every post reports 0 parts, as a
        /// bot removed from the group or a missing channel config would.
        refuse: AtomicBool,
    }

    impl RecordingPoster {
        fn posts(&self) -> Vec<(TenantId, ChannelType, String, String)> {
            self.posts.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl GroupChatPoster for RecordingPoster {
        async fn post(
            &self,
            tenant_id: TenantId,
            channel_type: ChannelType,
            chat_id: &str,
            text: &str,
        ) -> usize {
            self.posts.lock().unwrap().push((
                tenant_id,
                channel_type,
                chat_id.to_owned(),
                text.to_owned(),
            ));
            usize::from(!self.refuse.load(Ordering::SeqCst))
        }
    }

    /// Records every notification the pipeline hands to a linked chat channel.
    #[derive(Default)]
    struct RecordingSink {
        delivered: Mutex<Vec<(Uuid, String)>>,
    }

    impl RecordingSink {
        fn recipients(&self) -> Vec<Uuid> {
            self.delivered
                .lock()
                .unwrap()
                .iter()
                .map(|(user, _)| *user)
                .collect()
        }
    }

    #[async_trait]
    impl NotificationChannelSink for RecordingSink {
        async fn deliver(&self, request: &DispatchRequest) -> usize {
            self.delivered
                .lock()
                .unwrap()
                .push((request.user_id, request.body.clone()));
            1
        }
    }

    // ── Fixtures ────────────────────────────────────────────────────────

    struct Harness {
        resources: Arc<ServerContext>,
        runtime: Arc<dyn ToolRuntime>,
        service: NotificationService,
        sink: Arc<RecordingSink>,
        poster: Arc<RecordingPoster>,
    }

    impl Harness {
        async fn new() -> Self {
            let resources = create_test_server_resources().await.unwrap();
            let sink = Arc::new(RecordingSink::default());
            let service = match &*resources.agent.database {
                Database::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
                #[cfg(feature = "postgresql")]
                Database::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
            }
            .with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
                Arc::clone(&resources.common.repos),
                Arc::clone(&resources.mcp.messaging_strings_registry),
            )))
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>);
            let runtime: Arc<dyn ToolRuntime> = Arc::clone(&resources) as Arc<dyn ToolRuntime>;
            Self {
                resources,
                runtime,
                service,
                sink,
                poster: Arc::new(RecordingPoster::default()),
            }
        }

        async fn tick(&self, now: DateTime<Utc>) -> DigestTickOutcome {
            tick(
                &self.resources,
                &self.runtime,
                Some(&self.service),
                Some(self.poster.as_ref() as &dyn GroupChatPoster),
                now,
            )
            .await
            .unwrap()
        }

        /// An active user with a display name, a locale and maybe a zone.
        async fn person(&self, name: &str, locale: &str, zone: Option<&str>) -> Uuid {
            let email = format!("{}@digest.example.com", Uuid::new_v4());
            let mut user = User::new(email, "not-a-login".to_owned(), Some(name.to_owned()));
            user.user_status = UserStatus::Active;
            let id = user.id;
            let users = &self.resources.common.repos.users;
            users.create(&user).await.unwrap();
            users.update_locale(id, locale).await.unwrap();
            if let Some(zone) = zone {
                users.set_timezone(id, zone).await.unwrap();
            }
            id
        }

        /// A tenant on a plan whose tier enables the weekly digest.
        async fn tenant(&self, owner: Uuid) -> TenantId {
            let id = TenantId::generate();
            self.resources
                .common
                .repos
                .tenants
                .create(&Tenant {
                    id,
                    name: format!("Digest tenant {id}"),
                    slug: format!("digest-{id}"),
                    domain: None,
                    plan: "professional".to_owned(),
                    owner_user_id: owner,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
                .await
                .unwrap();
            id
        }

        /// A group owned by the first of `members` under `tenant` (see
        /// [`seed_group`]).
        async fn group(
            &self,
            tenant: TenantId,
            name: &str,
            digest_mode: GroupDigestMode,
            chat: Option<(&str, &str)>,
            members: &[(Uuid, GroupRole, bool)],
        ) -> Uuid {
            seed_group(
                &self.resources.common.repos,
                tenant,
                name,
                digest_mode,
                chat,
                members,
            )
            .await
        }

        /// The digest rows stored for `user` under `tenant`, newest first.
        async fn digest_rows(&self, user: Uuid, tenant: TenantId) -> Vec<String> {
            let (rows, _, _) = self
                .service
                .list_notifications(user, CommTenantId(tenant.as_uuid()), 20, 0, None, false)
                .await
                .unwrap();
            rows.into_iter()
                .filter(|row| row.notification_type == "group_weekly_digest")
                .map(|row| row.body)
                .collect()
        }
    }

    /// A group owned by the first of `members` under `tenant`, sending its
    /// digest as `digest_mode` says, bound to `chat` when given, with each
    /// member's role and consent. Every group here names its mode: the
    /// default is off, which sends nothing.
    async fn seed_group(
        repos: &RepositoryRegistry,
        tenant: TenantId,
        name: &str,
        digest_mode: GroupDigestMode,
        chat: Option<(&str, &str)>,
        members: &[(Uuid, GroupRole, bool)],
    ) -> Uuid {
        let owner = members[0].0;
        let agent = repos
            .agents
            .create_system_agent(
                owner,
                tenant,
                &CreateSystemAgentRequest {
                    title: format!("Coach {name}"),
                    description: None,
                    system_prompt: "You are a test coach.".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: AgentVisibility::Global,
                },
            )
            .await
            .unwrap();
        let id = Uuid::new_v4();
        let now = Utc::now();
        repos
            .groups
            .create_group(
                tenant,
                &CoachingGroup {
                    id,
                    tenant_id: tenant.to_string(),
                    name: name.to_owned(),
                    description: None,
                    agent_id: agent.id.to_string(),
                    owner_id: owner,
                    coach_user_id: None,
                    peer_data_sharing: true,
                    // Mentions-only rooms still get the digest: the mode
                    // governs replies.
                    respond_mode: GroupRespondMode::Mentions,
                    digest_mode,
                    max_members: 20,
                    is_active: true,
                    channel_type: chat.map(|(channel, _)| channel.to_owned()),
                    channel_chat_id: chat.map(|(_, chat_id)| chat_id.to_owned()),
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        for (user_id, role, consent) in members {
            repos
                .groups
                .add_member(&GroupMember {
                    id: Uuid::new_v4(),
                    group_id: id,
                    user_id: *user_id,
                    tenant_id: tenant.to_string(),
                    role: *role,
                    peer_sharing_consent: *consent,
                    consent_given_at: now,
                    joined_at: now,
                    left_at: None,
                    display_name: None,
                })
                .await
                .unwrap();
        }
        id
    }

    // ── The group's chat ────────────────────────────────────────────────

    /// The Telegram digest that motivated this: posted once into the group's
    /// chat, in French, naming only the members who share — while both managers
    /// keep the full digest in the app and nothing reaches their DMs.
    #[tokio::test]
    async fn a_bound_group_gets_its_digest_in_its_chat_and_managers_keep_it_in_app() {
        let h = Harness::new().await;
        let alice = h
            .person("Alice Martin", "fr", Some("America/Toronto"))
            .await;
        let bruno = h.person("Bruno Roy", "fr", None).await;
        let zoe = h.person("Zoé Tremblay", "fr", None).await;
        let tenant = h.tenant(alice).await;
        h.group(
            tenant,
            "Les Rouleurs",
            GroupDigestMode::Chat,
            Some(("telegram", "-1005284201188")),
            &[
                (alice, GroupRole::Owner, true),
                (bruno, GroupRole::Admin, true),
                (zoe, GroupRole::Member, false),
            ],
        )
        .await;

        let outcome = h.tick(monday_morning_montreal()).await;
        assert_eq!(outcome.groups_reported, 1, "{outcome:?}");
        assert_eq!(outcome.room_posts, 1, "{outcome:?}");
        assert_eq!(outcome.dispatched, 2, "{outcome:?}");
        assert_eq!(outcome.errors, 0, "{outcome:?}");

        let posts = h.poster.posts();
        assert_eq!(posts.len(), 1, "one post into the group's chat");
        let (post_tenant, channel, chat_id, text) = &posts[0];
        assert_eq!(*post_tenant, tenant, "the group's tenant owns the channel");
        assert_eq!(*channel, ChannelType::Telegram);
        assert_eq!(chat_id, "-1005284201188");
        let opening = format!(
            "🔔 Récap hebdo : Les Rouleurs\n\n{}\n\n2/2 membres actifs cette semaine",
            french_scope(2, 3)
        );
        assert!(text.starts_with(&opening), "{text}");
        assert!(text.contains("• Alice Martin : 0,0 km"), "{text}");
        assert!(text.contains("• Bruno Roy : 0,0 km"), "{text}");
        assert!(
            !text.contains("Zoé"),
            "a member who does not share is never named: {text}"
        );
        assert!(!text.contains("3/"), "nor counted: {text}");

        assert!(
            h.sink.recipients().is_empty(),
            "no manager gets the digest again in a DM"
        );
        for manager in [alice, bruno] {
            let rows = h.digest_rows(manager, tenant).await;
            assert_eq!(rows.len(), 1, "one in-app digest per manager");
            assert!(rows[0].starts_with("3/3 membres actifs"), "{}", rows[0]);
            assert!(rows[0].contains("• Zoé Tremblay : 0,0 km"), "{}", rows[0]);
        }
        assert!(
            h.digest_rows(zoe, tenant).await.is_empty(),
            "members get the chat copy only"
        );

        // Later the same week: the week is closed, nothing goes out again.
        let tuesday = monday_morning_montreal() + Duration::hours(26);
        let again = h.tick(tuesday).await;
        assert_eq!(again.groups_reported, 0, "{again:?}");
        assert_eq!(h.poster.posts().len(), 1);
        assert_eq!(h.digest_rows(alice, tenant).await.len(), 1);
    }

    /// A group whose digest is off gets nothing — no post, no manager copy —
    /// and leaves its week unclaimed, so switching it on later that week
    /// still sends that week's digest. The chat group beside it proves the
    /// tick ran through the same tenant.
    #[tokio::test]
    async fn a_group_whose_digest_is_off_gets_nothing_and_claims_no_week() {
        let h = Harness::new().await;
        let quiet_owner = h.person("Olga Off", "fr", Some("America/Toronto")).await;
        let quiet_member = h.person("Paul Off", "fr", None).await;
        let chat_owner = h.person("Rita Chat", "fr", Some("America/Toronto")).await;
        let tenant = h.tenant(quiet_owner).await;
        let quiet = h
            .group(
                tenant,
                "Club Muet",
                GroupDigestMode::Off,
                Some(("telegram", "-100111")),
                &[
                    (quiet_owner, GroupRole::Owner, true),
                    (quiet_member, GroupRole::Member, true),
                ],
            )
            .await;
        let chatty = h
            .group(
                tenant,
                "Club Bavard",
                GroupDigestMode::Chat,
                Some(("telegram", "-100222")),
                &[(chat_owner, GroupRole::Owner, true)],
            )
            .await;

        let outcome = h.tick(monday_morning_montreal()).await;
        assert_eq!(outcome.tenants_eligible, 1, "{outcome:?}");
        assert_eq!(
            outcome.groups_reported, 1,
            "only the chat group: {outcome:?}"
        );
        assert_eq!(outcome.room_posts, 1, "{outcome:?}");
        assert_eq!(
            outcome.dispatched, 1,
            "only the chat group's owner: {outcome:?}"
        );
        let posts = h.poster.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts[0].2, "-100222",
            "the off group's chat is never posted to"
        );
        assert!(h.sink.recipients().is_empty());
        for person in [quiet_owner, quiet_member] {
            assert!(
                h.digest_rows(person, tenant).await.is_empty(),
                "nobody in the off group gets a digest"
            );
        }

        let week = week_key(next_monday());
        let groups = &h.resources.common.repos.groups;
        let now_ms = monday_morning_montreal().timestamp_millis();
        assert!(
            groups
                .claim_group_digest(tenant, quiet, &week, now_ms, 60_000)
                .await
                .unwrap(),
            "the off group's week was never claimed, so it is still free"
        );
        assert!(
            !groups
                .claim_group_digest(tenant, chatty, &week, now_ms, 60_000)
                .await
                .unwrap(),
            "the chat group's week was sent and closed"
        );
    }

    /// A managers-only group posts nothing into its chat, however good the
    /// binding: the owner and the admin get the full digest over every member
    /// on their own channels and in the app, and a plain member gets nothing.
    #[tokio::test]
    async fn a_managers_group_posts_nothing_and_its_managers_hear_on_their_channels() {
        let h = Harness::new().await;
        let owner = h.person("Sam Owner", "fr", Some("America/Toronto")).await;
        let admin = h.person("Tess Admin", "fr", None).await;
        let member = h.person("Ugo Member", "fr", None).await;
        let tenant = h.tenant(owner).await;
        h.group(
            tenant,
            "Club Discret",
            GroupDigestMode::Managers,
            Some(("telegram", "-100333")),
            &[
                (owner, GroupRole::Owner, true),
                (admin, GroupRole::Admin, true),
                (member, GroupRole::Member, false),
            ],
        )
        .await;

        let outcome = h.tick(monday_morning_montreal()).await;
        assert_eq!(outcome.groups_reported, 1, "{outcome:?}");
        assert_eq!(outcome.room_posts, 0, "{outcome:?}");
        assert_eq!(outcome.dispatched, 2, "{outcome:?}");
        assert_eq!(outcome.errors, 0, "{outcome:?}");
        assert!(
            h.poster.posts().is_empty(),
            "nothing is posted into a managers-only group's chat"
        );

        let mut reached = h.sink.recipients();
        reached.sort_unstable();
        let mut managers = vec![owner, admin];
        managers.sort_unstable();
        assert_eq!(
            reached, managers,
            "each manager hears on their own channels"
        );
        for manager in [owner, admin] {
            let rows = h.digest_rows(manager, tenant).await;
            assert_eq!(rows.len(), 1, "one in-app digest per manager");
            assert!(rows[0].starts_with("3/3 membres actifs"), "{}", rows[0]);
            assert!(
                rows[0].contains("• Ugo Member : 0,0 km"),
                "the managers' copy covers every member, sharing or not: {}",
                rows[0]
            );
        }
        assert!(h.digest_rows(member, tenant).await.is_empty());
    }

    /// A group with no chat, or bound where a proactive message bounces, keeps
    /// the managers' notification on their own linked channels.
    #[tokio::test]
    async fn a_group_without_a_postable_chat_reaches_its_managers_channels() {
        let h = Harness::new().await;
        let web_owner = h.person("Carla Web", "fr", Some("America/Toronto")).await;
        let wa_owner = h.person("Dan Whats", "en", Some("America/Toronto")).await;
        let member = h.person("Eve Member", "fr", None).await;
        let tenant = h.tenant(web_owner).await;
        h.group(
            tenant,
            "Web Club",
            GroupDigestMode::Chat,
            None,
            &[
                (web_owner, GroupRole::Owner, true),
                (member, GroupRole::Member, true),
            ],
        )
        .await;
        h.group(
            tenant,
            "WhatsApp Club",
            GroupDigestMode::Chat,
            Some(("whatsapp", "120363")),
            &[(wa_owner, GroupRole::Owner, true)],
        )
        .await;

        let outcome = h.tick(monday_morning_montreal()).await;
        assert_eq!(outcome.groups_reported, 2, "{outcome:?}");
        assert_eq!(outcome.room_posts, 0, "{outcome:?}");
        assert!(h.poster.posts().is_empty(), "nothing is posted into a chat");
        let mut reached = h.sink.recipients();
        reached.sort_unstable();
        let mut owners = vec![web_owner, wa_owner];
        owners.sort_unstable();
        assert_eq!(reached, owners, "each owner hears on their linked channels");
    }

    /// With nobody sharing, the chat reads the title and the invitation to
    /// share — no empty summary, no all-clear about nobody.
    #[tokio::test]
    async fn when_nobody_shares_the_chat_reads_only_the_scope_line() {
        let h = Harness::new().await;
        let owner = h.person("Félix Owner", "fr", Some("America/Toronto")).await;
        let member = h.person("Gina Member", "fr", None).await;
        let tenant = h.tenant(owner).await;
        h.group(
            tenant,
            "Solo Club",
            GroupDigestMode::Chat,
            Some(("discord", "998877")),
            &[
                (owner, GroupRole::Owner, false),
                (member, GroupRole::Member, false),
            ],
        )
        .await;

        h.tick(monday_morning_montreal()).await;
        let posts = h.poster.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts[0].3,
            format!("🔔 Récap hebdo : Solo Club\n\n{}", french_scope(0, 2))
        );
    }

    /// Two members read English on this channel (one by a per-channel
    /// override), one reads French: the chat reads English even though the
    /// owner reads French.
    #[tokio::test]
    async fn the_chat_reads_the_language_most_members_read() {
        let h = Harness::new().await;
        let owner = h
            .person("Hélène Owner", "fr", Some("America/Toronto"))
            .await;
        let english = h.person("Ian English", "en", None).await;
        let switched = h.person("Jade Switched", "fr", None).await;
        let tenant = h.tenant(owner).await;
        h.group(
            tenant,
            "Mixed Club",
            GroupDigestMode::Chat,
            Some(("telegram", "-100555")),
            &[
                (owner, GroupRole::Owner, true),
                (english, GroupRole::Member, true),
                (switched, GroupRole::Member, true),
            ],
        )
        .await;
        let messaging: &dyn MessagingRepository = &*h.resources.common.repos.messaging;
        messaging
            .create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: tenant,
                user_id: &switched.to_string(),
                channel_type: "telegram",
                channel_user_id: "4242",
                display_name: Some("Jade"),
            })
            .await
            .unwrap();
        messaging
            .set_channel_link_locale(tenant, &switched.to_string(), "telegram", Some("en"))
            .await
            .unwrap();

        h.tick(monday_morning_montreal()).await;
        let posts = h.poster.posts();
        assert_eq!(posts.len(), 1);
        let text = &posts[0].3;
        assert!(
            text.starts_with("🔔 Weekly recap: Mixed Club\n\n3/3 members active this week"),
            "everyone shares, so there is no scope line: {text}"
        );
    }

    /// Outside the slot nothing is sent, and a group with no zone on file
    /// waits for 12:00 UTC.
    #[tokio::test]
    async fn nothing_goes_out_before_the_slot() {
        let h = Harness::new().await;
        let owner = h.person("Kim NoZone", "fr", None).await;
        let tenant = h.tenant(owner).await;
        h.group(
            tenant,
            "No Zone Club",
            GroupDigestMode::Chat,
            Some(("slack", "C0NOZONE")),
            &[(owner, GroupRole::Owner, true)],
        )
        .await;

        let before = next_monday_utc(11, 59);
        assert_eq!(h.tick(before).await.groups_reported, 0);
        assert!(h.poster.posts().is_empty());

        let noon = next_monday_utc(12, 0);
        assert_eq!(h.tick(noon).await.room_posts, 1);
        assert_eq!(h.poster.posts()[0].1, ChannelType::Slack);
    }

    // ── The delivery ledger ─────────────────────────────────────────────

    /// A week is claimed once; a crashed claim is retried only after its
    /// lease; a finished week is never claimed again; the next week is new.
    /// Runs on `SQLite` here; the `PostgreSQL` lane runs the same statements.
    #[tokio::test]
    async fn a_group_week_is_claimed_once() {
        let db = create_test_db().await;
        let repos = db.repositories();
        let (owner, tenant) = seed_user(&db).await;
        let group = seed_group(
            &repos,
            tenant,
            "Ledger Club",
            GroupDigestMode::Chat,
            None,
            &[(owner, GroupRole::Owner, true)],
        )
        .await;
        let groups = &repos.groups;
        let lease = 60_000;
        let t0 = 1_000_000;

        assert!(groups
            .claim_group_digest(tenant, group, "2026-W40", t0, lease)
            .await
            .unwrap());
        assert!(
            !groups
                .claim_group_digest(tenant, group, "2026-W40", t0 + 1, lease)
                .await
                .unwrap(),
            "a second claim while the first is held is refused"
        );
        assert!(
            groups
                .claim_group_digest(tenant, group, "2026-W40", t0 + lease, lease)
                .await
                .unwrap(),
            "a claim whose holder died is taken over once its lease lapses"
        );
        groups
            .finish_group_digest(tenant, group, "2026-W40", t0 + lease + 5)
            .await
            .unwrap();
        assert!(
            !groups
                .claim_group_digest(tenant, group, "2026-W40", t0 + 10 * lease, lease)
                .await
                .unwrap(),
            "a finished week is never claimed again"
        );
        assert!(
            groups
                .claim_group_digest(tenant, group, "2026-W41", t0 + 10 * lease, lease)
                .await
                .unwrap(),
            "the next week is its own claim"
        );
        let other_tenant = TenantId::generate();
        assert!(
            groups
                .claim_group_digest(other_tenant, group, "2026-W40", t0, lease)
                .await
                .unwrap(),
            "the ledger is keyed by tenant too"
        );
    }

    // ── The room copy's parameters ──────────────────────────────────────

    fn snapshot(
        user_id: Uuid,
        name: &str,
        km: f64,
        form: Option<(f64, f64)>,
    ) -> MemberFitnessSnapshot {
        MemberFitnessSnapshot {
            user_id,
            display_name: name.to_owned(),
            ctl: form.map(|(ctl, _)| ctl),
            atl: None,
            tsb: form.map(|(_, tsb)| tsb),
            weekly_volume_km: km,
            previous_week_volume_km: Some(km),
            weekly_activity_count: 3,
            weekly_duration_seconds: 3600,
            primary_sport: None,
            vdot: None,
            overtraining_risk: OvertrainingRiskLevel::Low,
            days_since_last_activity: Some(1),
            last_activity_per_provider: HashMap::new(),
            recent_activities: Vec::new(),
            needs_reauth_providers: Vec::new(),
            served_stale: false,
            timezone: None,
            computed_at: Utc::now(),
        }
    }

    fn member(group_id: Uuid, user_id: Uuid, consent: bool) -> GroupMember {
        GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: "t".to_owned(),
            role: GroupRole::Member,
            peer_sharing_consent: consent,
            consent_given_at: Utc::now(),
            joined_at: Utc::now(),
            left_at: None,
            display_name: None,
        }
    }

    fn bare_group(peer_data_sharing: bool) -> CoachingGroup {
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: "t".to_owned(),
            name: "Les Rouleurs".to_owned(),
            description: None,
            agent_id: "a".to_owned(),
            owner_id: Uuid::new_v4(),
            coach_user_id: None,
            peer_data_sharing,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::Chat,
            max_members: 20,
            is_active: true,
            channel_type: Some("telegram".to_owned()),
            channel_chat_id: Some("-1".to_owned()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn render_fr(resources: &ServerContext, params: &Value) -> String {
        let empty = Map::new();
        NotificationTextRenderer::new(&resources.mcp.messaging_strings_registry, "fr").channel_text(
            NotificationEvent::GroupWeeklyDigest,
            params.as_object().unwrap_or(&empty),
        )
    }

    /// A member who does not share, however extreme their week, changes
    /// nothing in the room copy but the roster count.
    #[tokio::test]
    async fn a_member_who_does_not_share_leaves_the_room_copy_unchanged() {
        let resources = create_test_server_resources().await.unwrap();
        let service = resources.common.group_service.as_ref();
        let group = bare_group(true);
        let (phil, marie, secret) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let shared = vec![
            snapshot(phil, "Phil", 180.5, Some((100.0, 12.0))),
            snapshot(marie, "Marie", 120.0, None),
        ];
        let mut everyone = shared.clone();
        everyone.push(snapshot(secret, "Sécret", 999.9, Some((100.0, -80.0))));

        let two = [member(group.id, phil, true), member(group.id, marie, true)];
        let three = [
            member(group.id, phil, true),
            member(group.id, marie, true),
            member(group.id, secret, false),
        ];
        let mut without = room_digest_params(service, &group, &two, &shared);
        let mut with = room_digest_params(service, &group, &three, &everyone);
        assert_eq!(with["shared_members"], 2);
        assert_eq!(with["roster_members"], 3);
        without.as_object_mut().unwrap().remove("roster_members");
        with.as_object_mut().unwrap().remove("roster_members");
        assert_eq!(with, without, "the non-sharing member moved nothing");

        let text = render_fr(
            &resources,
            &room_digest_params(service, &group, &three, &everyone),
        );
        assert!(!text.contains("Sécret"), "{text}");
        assert!(!text.contains("999"), "{text}");
        assert!(!text.contains("-80"), "{text}");
        assert!(text.contains(&french_scope(2, 3)), "{text}");
    }

    /// The all-clear is said of the members who share, so it appears exactly
    /// when none of them is flagged — whatever a withheld member's week was.
    #[tokio::test]
    async fn the_room_all_clear_is_true_of_the_members_who_share() {
        let resources = create_test_server_resources().await.unwrap();
        let service = resources.common.group_service.as_ref();
        let group = bare_group(true);
        let (quiet, flagged, withheld) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let snapshots = vec![
            snapshot(quiet, "Quiet", 40.0, None),
            snapshot(flagged, "Tired", 40.0, Some((100.0, -45.0))),
            snapshot(withheld, "Withheld", 40.0, Some((100.0, -45.0))),
        ];

        let only_quiet_shares = [
            member(group.id, quiet, true),
            member(group.id, flagged, false),
            member(group.id, withheld, false),
        ];
        let text = render_fr(
            &resources,
            &room_digest_params(service, &group, &only_quiet_shares, &snapshots),
        );
        assert!(text.ends_with(FRENCH_ALL_CLEAR), "{text}");
        assert!(
            text.contains(&french_scope(1, 3)),
            "the all-clear is scoped: {text}"
        );

        let a_flagged_member_shares = [
            member(group.id, quiet, true),
            member(group.id, flagged, true),
            member(group.id, withheld, false),
        ];
        let text = render_fr(
            &resources,
            &room_digest_params(service, &group, &a_flagged_member_shares, &snapshots),
        );
        assert!(!text.contains(FRENCH_ALL_CLEAR), "{text}");
        assert!(text.contains("• Tired : forme à -45 %"), "{text}");
        assert!(!text.contains("Withheld"), "{text}");
    }

    /// The admin kill switch withholds everyone, whatever they consented to.
    #[tokio::test]
    async fn the_kill_switch_withholds_every_member() {
        let resources = create_test_server_resources().await.unwrap();
        let service = resources.common.group_service.as_ref();
        let group = bare_group(false);
        let phil = Uuid::new_v4();
        let params = room_digest_params(
            service,
            &group,
            &[member(group.id, phil, true)],
            &[snapshot(phil, "Phil", 50.0, None)],
        );
        assert_eq!(
            render_fr(&resources, &params),
            format!("🔔 Récap hebdo : Les Rouleurs\n\n{}", french_scope(0, 1))
        );
    }

    // ── The production poster ───────────────────────────────────────────

    /// A long digest is split under Discord's ceiling and sent in order to the
    /// chat itself; people's names travel as characters, and on Slack a name
    /// shaped like a mention is rendered inert.
    #[tokio::test]
    async fn the_poster_escapes_names_and_splits_to_the_channel_ceiling() {
        let discord = Arc::new(CapturingChannel::for_channel(ChannelType::Discord));
        let poster =
            ServerGroupChatPoster::with_resolver(Arc::new(FakeResolver::new(discord.clone())));
        let tenant = TenantId::generate();
        let line = "• jean_f*x @everyone : 42,0 km (semaine précédente : 40,0 km)\n";
        let text = format!("🔔 Récap hebdo : Les Rouleurs\n\n{}", line.repeat(120));

        let parts = poster
            .post(tenant, ChannelType::Discord, "998877", &text)
            .await;
        let sent = discord.sent.lock().unwrap().clone();
        assert_eq!(parts, sent.len());
        assert!(
            parts >= 3,
            "{} characters in {parts} parts",
            text.chars().count()
        );
        for message in &sent {
            assert_eq!(message.recipient_id, "998877");
            assert!(message.reply_to.is_none() && message.thread_id.is_none());
            assert!(
                matches!(message.content, MessageContent::RichText { .. }),
                "the digest travels as rich text: {:?}",
                message.content
            );
            // Discord counts the content its renderer sends, backslashes included.
            let rendered = DiscordRenderer.render(message).unwrap();
            let content = rendered["content"].as_str().unwrap();
            assert!(
                content.chars().count() <= 2000,
                "{} characters once rendered",
                content.chars().count()
            );
            assert!(
                !content.contains("@everyone"),
                "a name cannot ping the server: {content}"
            );
            assert!(content.contains("@\u{200B}everyone"), "{content}");
        }

        let rendered = DiscordRenderer.render(&sent[0]).unwrap();
        let content = rendered["content"].as_str().unwrap();
        assert!(
            content.contains(r"jean\_f\*x"),
            "a name's markdown characters reach Discord escaped, so they print: {content}"
        );

        let slack = Arc::new(CapturingChannel::for_channel(ChannelType::Slack));
        let poster =
            ServerGroupChatPoster::with_resolver(Arc::new(FakeResolver::new(slack.clone())));
        let delivered = poster
            .post(
                tenant,
                ChannelType::Slack,
                "C0123",
                "• <!channel> & jean_f : 1,0 km",
            )
            .await;
        assert_eq!(delivered, 1);
        let payload = SlackRenderer
            .render(&slack.sent.lock().unwrap()[0])
            .unwrap();
        let rendered = payload["blocks"][0]["text"]["text"].as_str().unwrap();
        assert!(
            rendered.contains("&lt;!channel&gt; &amp; jean_f"),
            "{rendered}"
        );
    }

    /// A chat that takes none of the post (the bot was removed, the channel
    /// config is gone) counts as an error, and its managers hear about the
    /// digest on their own channels instead of only in the app.
    #[tokio::test]
    async fn a_refused_chat_post_reaches_the_managers_channels_and_counts_as_an_error() {
        let h = Harness::new().await;
        h.poster.refuse.store(true, Ordering::SeqCst);
        let owner = h.person("Hugo Owner", "fr", Some("America/Toronto")).await;
        let member = h.person("Iris Member", "fr", None).await;
        let tenant = h.tenant(owner).await;
        h.group(
            tenant,
            "Refused Club",
            GroupDigestMode::Chat,
            Some(("telegram", "-100999")),
            &[
                (owner, GroupRole::Owner, true),
                (member, GroupRole::Member, true),
            ],
        )
        .await;

        let outcome = h.tick(monday_morning_montreal()).await;
        assert_eq!(h.poster.posts().len(), 1, "the chat was tried once");
        assert_eq!(outcome.room_posts, 0, "{outcome:?}");
        assert_eq!(outcome.errors, 1, "{outcome:?}");
        assert_eq!(
            h.sink.recipients(),
            vec![owner],
            "the owner hears on their linked channels"
        );
        assert_eq!(h.digest_rows(owner, tenant).await.len(), 1);
    }

    /// The poster stops at the first part a chat refuses: later parts are
    /// never sent, so the chat never reads a digest with its opening missing.
    #[tokio::test]
    async fn the_poster_stops_at_the_first_refused_part() {
        let discord = Arc::new(FailingChannel::for_channel(ChannelType::Discord));
        let poster =
            ServerGroupChatPoster::with_resolver(Arc::new(FakeResolver::new(discord.clone())));
        let text = format!(
            "🔔 Récap hebdo : Les Rouleurs\n\n{}",
            "• Phil : 42,0 km (semaine précédente : 40,0 km)\n".repeat(120)
        );

        let delivered = poster
            .post(TenantId::generate(), ChannelType::Discord, "998877", &text)
            .await;
        assert_eq!(delivered, 0);
        assert_eq!(
            discord.attempts.lock().unwrap().len(),
            1,
            "a multi-part digest stops after the refused first part"
        );
    }
}
