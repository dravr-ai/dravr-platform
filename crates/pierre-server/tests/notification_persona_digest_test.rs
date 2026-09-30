// ABOUTME: Every persona digest cadence returns exactly the pushes the armed policy withheld
// ABOUTME: Weekly, daily and per-athlete on the scheduler tick; per-session when a training session lands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona digests end-to-end.
//!
//! An armed persona floor persists a push above it with the `persona_gated`
//! marker instead of delivering it. Each cadence the contracts name must hand
//! those held rows back — and only them, and each exactly once:
//!
//! - `weekly` and `daily` on the scheduler tick, once their period has passed
//!   since the previous digest;
//! - `per_athlete` on the tick, one digest per athlete the held rows concern;
//! - `per_session` never on the tick, only when the athlete's next training
//!   session lands in the activity cache.
//!
//! Every digest is asserted by content: the ids of the rows it returns (read
//! from the server's record of its claim), the count its sentence carries,
//! the athlete it names. A digest that returned nothing, returned everything,
//! or returned a row twice fails — including when the athlete deleted the
//! earlier digest, when two sessions land at once, and when the pipeline
//! suppressed a digest.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "client-notifications")]
mod digest_tests {
    use std::collections::BTreeSet;
    use std::slice;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration as StdDuration, Instant};

    use async_trait::async_trait;
    use chrono::{DateTime, Duration, Utc};
    use serde_json::{json, Value};
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources, create_test_tenant};
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
    use pierre_core::feature_flags::FeatureKey;
    use pierre_core::models::{ActivityBuilder, CoachingPersona, SportType, TenantId};
    use pierre_database::backends::factory::{Database, DatabaseBackend};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::events::{event_params, SubjectAthlete};
    use pierre_notifications::models::{
        CreateNotificationParams, Notification, NotificationCategory,
        UpsertNotificationPreferenceParams,
    };
    use pierre_notifications::{
        triggers, DispatchOutcome, DispatchRequest, NotificationChannelSink, NotificationEvent,
        NotificationService, PushTier, TenantId as CommTenantId, PERSONA_GATED_DATA_KEY,
    };
    use pierre_providers::core::ActivityQueryParams;
    use pierre_services::notification_digest_scheduler::{
        session_landed, tick, DIGEST_BATCH_PARAM, PERSONA_DIGEST_TYPE,
    };
    use pierre_services::persona_notification_policy_gate::PersonaNotificationPolicyGate;
    use pierre_tool_runtime::activity_fetch::write_through::write_through_served_window;
    use pierre_tool_runtime::runtime::ToolRuntime;

    const CASUAL_P0_WEEKLY: &str = r"
version: 1
personas:
  casual:
    notification:
      tier_floor: P0
      digest: weekly
";

    const ENTHUSIAST_P1_DAILY: &str = r"
version: 1
personas:
  enthusiast:
    notification:
      tier_floor: P1
      digest: daily
";

    const POWER_P2_PER_SESSION: &str = r"
version: 1
personas:
  power_athlete:
    notification:
      tier_floor: P2
      digest: per_session
";

    /// The shipped coach contract names a P2 floor; P1 here so the coach's
    /// P2 delegation notices are the ones held — the notices that name the
    /// member they concern.
    const COACH_P1_PER_ATHLETE: &str = r"
version: 1
personas:
  coach:
    notification:
      tier_floor: P1
      digest: per_athlete
";

    /// A sink that records deliveries, standing for one linked chat channel,
    /// so "nothing went out until the digest" is asserted by content.
    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl NotificationChannelSink for RecordingSink {
        async fn deliver(&self, request: &DispatchRequest) -> usize {
            self.seen
                .lock()
                .unwrap()
                .push(request.notification_type.clone());
            1
        }
    }

    impl RecordingSink {
        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn notification_service(db: &Database) -> NotificationService {
        match db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                NotificationService::from_sqlite(sqlite.pool().clone())
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                NotificationService::from_postgres(pg.pool().clone())
            }
        }
    }

    /// The service the digests read their policy through: the real persona
    /// gate over `overlay`, and a recording channel sink.
    fn gated_service(
        resources: &ServerContext,
        overlay: &str,
    ) -> (NotificationService, Arc<RecordingSink>) {
        let registry = Arc::new(PersonaContractRegistry::new());
        registry.apply_overlay(overlay).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let service = notification_service(&resources.agent.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>)
            .with_policy_gate(Arc::new(PersonaNotificationPolicyGate::new(
                Arc::clone(&resources.common.repos),
                registry,
            )));
        (service, sink)
    }

    /// Seed one active user with `persona` in `locale`, armed or not.
    async fn seed_user(
        resources: &ServerContext,
        email: &str,
        persona: CoachingPersona,
        locale: &str,
        armed: bool,
    ) -> (Uuid, TenantId) {
        let (user, _token) = create_test_tenant(resources, email).await.unwrap();
        let repos = &resources.common.repos;
        repos
            .users
            .set_coaching_persona(user.id, persona)
            .await
            .unwrap();
        repos.users.update_locale(user.id, locale).await.unwrap();
        if armed {
            repos
                .feature_flags
                .set_user_override(user.id, FeatureKey::PersonaNotificationPolicy, true, None)
                .await
                .unwrap();
        }
        let tenant = repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;
        (user.id, tenant)
    }

    fn comm(tenant: TenantId) -> CommTenantId {
        CommTenantId(tenant.as_uuid())
    }

    /// Dispatch one `tier` push the armed gate must hold; returns the id of
    /// the row it persisted instead.
    async fn hold(
        service: &NotificationService,
        user_id: Uuid,
        tenant: TenantId,
        notification_type: &str,
        tier: PushTier,
    ) -> Uuid {
        let outcome = service
            .dispatch_with_tier(
                &DispatchRequest {
                    user_id,
                    tenant_id: comm(tenant),
                    category: NotificationCategory::Training,
                    notification_type: notification_type.to_owned(),
                    title: "Training load elevated".to_owned(),
                    body: "held for the digest".to_owned(),
                    data: Some(json!({ "screen": "coach", "action": "chat", "id": "conv-load-1" })),
                    image_url: None,
                    actions: None,
                    bypass_frequency_cap: false,
                },
                tier,
            )
            .await
            .unwrap();
        let DispatchOutcome::PersistedNoDevices { notification_id } = outcome else {
            panic!("the armed gate must hold a {tier} push: {outcome:?}");
        };
        notification_id
    }

    async fn rows(
        service: &NotificationService,
        user_id: Uuid,
        tenant: TenantId,
    ) -> Vec<Notification> {
        let (rows, _, _) = service
            .list_notifications(user_id, comm(tenant), 200, 0, None, false)
            .await
            .unwrap();
        rows
    }

    async fn digests_of(
        service: &NotificationService,
        user_id: Uuid,
        tenant: TenantId,
        event: NotificationEvent,
    ) -> Vec<Notification> {
        rows(service, user_id, tenant)
            .await
            .into_iter()
            .filter(|n| n.notification_type == event.wire())
            .collect()
    }

    /// The claim a digest names: the id its returned rows are recorded under.
    fn batch_of(digest: &Notification) -> Uuid {
        event_params(digest.data.as_ref())
            .and_then(|p| p.get(DIGEST_BATCH_PARAM))
            .and_then(Value::as_str)
            .expect("a digest names the claim holding the rows it returns")
            .parse()
            .unwrap()
    }

    /// The held-row ids a digest returned, read from the server's own record
    /// of its claim — the record a later digest excludes them by.
    async fn returned(db: &Database, digest: &Notification) -> BTreeSet<Uuid> {
        let batch = batch_of(digest);
        // notification_id is TEXT on SQLite and UUID on PostgreSQL, the type of
        // the notifications.id it references on each engine.
        match db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                let rows: Vec<String> = sqlx::query_scalar(
                    "SELECT notification_id FROM persona_digest_returns WHERE batch_id = ?",
                )
                .bind(batch.to_string())
                .fetch_all(sqlite.pool())
                .await
                .unwrap();
                rows.iter().map(|id| id.parse().unwrap()).collect()
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => sqlx::query_scalar::<_, Uuid>(
                "SELECT notification_id FROM persona_digest_returns WHERE batch_id = $1",
            )
            .bind(batch)
            .fetch_all(pg.pool())
            .await
            .unwrap()
            .into_iter()
            .collect(),
        }
    }

    fn item_count(digest: &Notification) -> Option<u64> {
        event_params(digest.data.as_ref())
            .and_then(|p| p.get("item_count"))
            .and_then(Value::as_u64)
    }

    fn ids(list: &[Uuid]) -> BTreeSet<Uuid> {
        list.iter().copied().collect()
    }

    /// Move the time a digest returned its rows, standing in for the time a
    /// cadence waits between two digests.
    async fn backdate(db: &Database, digest: &Notification, to: DateTime<Utc>) {
        let batch = batch_of(digest);
        match db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                sqlx::query(
                    "UPDATE persona_digest_returns SET returned_at_ms = ? WHERE batch_id = ?",
                )
                .bind(to.timestamp_millis())
                .bind(batch.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                sqlx::query(
                    "UPDATE persona_digest_returns SET returned_at_ms = $1 WHERE batch_id = $2",
                )
                .bind(to.timestamp_millis())
                .bind(batch)
                .execute(pg.pool())
                .await
                .unwrap();
            }
        }
    }

    // ════════════════════════════════════════════════════════════════
    // weekly
    // ════════════════════════════════════════════════════════════════

    /// One tick sends one localized digest per armed weekly user returning
    /// exactly their held rows; a second immediate tick sends nothing.
    #[tokio::test]
    async fn weekly_digest_returns_exactly_the_held_rows_once() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, sink) = gated_service(&resources, CASUAL_P0_WEEKLY);

        let (fr_user, fr_tenant) = seed_user(
            &resources,
            "digest_fr@example.com",
            CoachingPersona::Casual,
            "fr",
            true,
        )
        .await;
        let (en_user, en_tenant) = seed_user(
            &resources,
            "digest_en@example.com",
            CoachingPersona::Casual,
            "en",
            true,
        )
        .await;

        let fr_held = [
            hold(
                &service,
                fr_user,
                fr_tenant,
                "training_load_alert",
                PushTier::P2,
            )
            .await,
            hold(
                &service,
                fr_user,
                fr_tenant,
                "low_recovery_score",
                PushTier::P2,
            )
            .await,
        ];
        let en_held = [
            hold(
                &service,
                en_user,
                en_tenant,
                "training_load_alert",
                PushTier::P2,
            )
            .await,
            hold(
                &service,
                en_user,
                en_tenant,
                "low_recovery_score",
                PushTier::P2,
            )
            .await,
            hold(&service, en_user, en_tenant, "coach_message", PushTier::P1).await,
        ];
        assert!(sink.seen().is_empty(), "a held push reaches no channel");

        let strings = MessagingStringsRegistry::new();
        let outcome = tick(&repos, &service, &strings).await.unwrap();
        assert_eq!(
            outcome.digests_sent, 2,
            "one digest per armed weekly user: {outcome:?}"
        );
        assert_eq!(outcome.errors, 0, "no per-user errors: {outcome:?}");
        assert_eq!(sink.seen(), ["persona_digest", "persona_digest"]);

        let fr_digests = digests_of(
            &service,
            fr_user,
            fr_tenant,
            NotificationEvent::PersonaDigest,
        )
        .await;
        assert_eq!(fr_digests.len(), 1, "exactly one digest for the fr user");
        assert_eq!(fr_digests[0].title, "Ton récap hebdo de notifications");
        assert!(
            fr_digests[0].body.starts_with("2 notification(s)"),
            "fr body carries the item count: {}",
            fr_digests[0].body
        );
        assert_eq!(returned(db, &fr_digests[0]).await, ids(&fr_held));
        assert_eq!(item_count(&fr_digests[0]), Some(2));

        let en_digests = digests_of(
            &service,
            en_user,
            en_tenant,
            NotificationEvent::PersonaDigest,
        )
        .await;
        assert_eq!(en_digests.len(), 1, "exactly one digest for the en user");
        assert_eq!(en_digests[0].title, "Your weekly notification digest");
        assert!(
            en_digests[0].body.starts_with("3 notification(s)"),
            "en body carries the item count: {}",
            en_digests[0].body
        );
        assert_eq!(returned(db, &en_digests[0]).await, ids(&en_held));

        // Restart/retry safety: every held row is already returned.
        let second = tick(&repos, &service, &strings).await.unwrap();
        assert_eq!(
            second.digests_sent, 0,
            "nothing new held ⇒ no digest: {second:?}"
        );
        assert_eq!(
            digests_of(
                &service,
                fr_user,
                fr_tenant,
                NotificationEvent::PersonaDigest
            )
            .await
            .len(),
            1
        );
        assert_eq!(PERSONA_DIGEST_TYPE, "persona_digest");
    }

    /// A weekly digest waits out its week: a row held the next day is not
    /// sent the next day, and goes out alone once seven days have passed.
    #[tokio::test]
    async fn weekly_digest_waits_out_the_week() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, _sink) = gated_service(&resources, CASUAL_P0_WEEKLY);
        let (user, tenant) = seed_user(
            &resources,
            "digest_week@example.com",
            CoachingPersona::Casual,
            "en",
            true,
        )
        .await;
        let strings = MessagingStringsRegistry::new();

        let first = hold(&service, user, tenant, "training_load_alert", PushTier::P2).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        let digest = digests_of(&service, user, tenant, NotificationEvent::PersonaDigest).await;
        assert_eq!(returned(db, &digest[0]).await, ids(&[first]));

        let later = hold(&service, user, tenant, "low_recovery_score", PushTier::P2).await;
        backdate(db, &digest[0], Utc::now() - Duration::days(1)).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            0,
            "one day after the previous digest a weekly digest is not due"
        );

        backdate(db, &digest[0], Utc::now() - Duration::days(7)).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        let digests = digests_of(&service, user, tenant, NotificationEvent::PersonaDigest).await;
        assert_eq!(digests.len(), 2);
        let newest = digests.iter().find(|d| d.id != digest[0].id).unwrap();
        assert_eq!(
            returned(db, newest).await,
            ids(&[later]),
            "the week's digest returns only what was held since the last one"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // daily
    // ════════════════════════════════════════════════════════════════

    /// A daily digest batches the day's held rows, then waits a day.
    #[tokio::test]
    async fn daily_digest_batches_once_a_day() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, sink) = gated_service(&resources, ENTHUSIAST_P1_DAILY);
        let (user, tenant) = seed_user(
            &resources,
            "digest_daily@example.com",
            CoachingPersona::Enthusiast,
            "en",
            true,
        )
        .await;
        let strings = MessagingStringsRegistry::new();

        let held = [
            hold(&service, user, tenant, "training_load_alert", PushTier::P2).await,
            hold(&service, user, tenant, "personal_record", PushTier::P3).await,
        ];
        // P1 is at the enthusiast floor: it goes straight out, never held.
        service
            .dispatch_with_tier(
                &DispatchRequest {
                    user_id: user,
                    tenant_id: comm(tenant),
                    category: NotificationCategory::Coach,
                    notification_type: "coach_message".to_owned(),
                    title: "Coach".to_owned(),
                    body: "delivered".to_owned(),
                    data: None,
                    image_url: None,
                    actions: None,
                    bypass_frequency_cap: true,
                },
                PushTier::P1,
            )
            .await
            .unwrap();
        assert_eq!(sink.seen(), ["coach_message"], "only the P1 push went out");

        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        assert_eq!(sink.seen(), ["coach_message", "persona_daily_digest"]);
        let digests = digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaDailyDigest,
        )
        .await;
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].title, "Your daily notification digest");
        assert!(
            digests[0].body.starts_with("2 notification(s)"),
            "the body counts the held rows: {}",
            digests[0].body
        );
        assert_eq!(
            returned(db, &digests[0]).await,
            ids(&held),
            "the delivered P1 is not in it"
        );

        let first_digest = digests[0].id;
        let next = hold(&service, user, tenant, "low_recovery_score", PushTier::P2).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            0,
            "a second digest the same day is not due"
        );
        backdate(db, &digests[0], Utc::now() - Duration::days(1)).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        let digests = digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaDailyDigest,
        )
        .await;
        assert_eq!(digests.len(), 2);
        let newest = digests.iter().find(|d| d.id != first_digest).unwrap();
        assert_eq!(returned(db, newest).await, ids(&[next]));
    }

    // ════════════════════════════════════════════════════════════════
    // per_athlete
    // ════════════════════════════════════════════════════════════════

    /// A coach's held notices come back as one digest per athlete they
    /// concern, each naming that athlete and returning only that athlete's
    /// notices, plus one daily digest for the coach's own.
    #[tokio::test]
    async fn per_athlete_digest_rolls_up_one_digest_per_athlete() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, sink) = gated_service(&resources, COACH_P1_PER_ATHLETE);
        let service = Arc::new(service);
        let (coach, tenant) = seed_user(
            &resources,
            "digest_coach@example.com",
            CoachingPersona::Coach,
            "en",
            true,
        )
        .await;
        let alice = Uuid::new_v4();
        let bruno = Uuid::new_v4();

        // The real coach-facing triggers, fired at P2 — above this coach's
        // P1 floor, so all three are held.
        triggers::trigger_delegation_confirmed(
            &service,
            coach,
            comm(tenant),
            alice,
            "Alice",
            "Crew",
        );
        triggers::trigger_delegation_declined(
            &service,
            coach,
            comm(tenant),
            bruno,
            "Bruno",
            "Crew",
        );
        triggers::trigger_delegation_off_roster(
            &service,
            coach,
            comm(tenant),
            alice,
            "Alice",
            "Crew",
        );
        let own = hold(&service, coach, tenant, "training_load_alert", PushTier::P2).await;

        let deadline = Instant::now() + StdDuration::from_secs(10);
        let held = loop {
            let held: Vec<Notification> = rows(&service, coach, tenant)
                .await
                .into_iter()
                .filter(|n| {
                    n.data
                        .as_ref()
                        .and_then(|d| d.get(PERSONA_GATED_DATA_KEY))
                        .is_some()
                })
                .collect();
            if held.len() == 4 {
                break held;
            }
            assert!(
                Instant::now() < deadline,
                "the triggers held {} of 4 rows",
                held.len()
            );
            sleep(StdDuration::from_millis(25)).await;
        };
        assert!(sink.seen().is_empty(), "every notice was held");
        let about = |athlete: Uuid| -> BTreeSet<Uuid> {
            held.iter()
                .filter(|n| {
                    SubjectAthlete::from_data(n.data.as_ref()).map(|s| s.id) == Some(athlete)
                })
                .map(|n| n.id)
                .collect()
        };
        assert_eq!(about(alice).len(), 2, "both of Alice's notices name her");
        assert_eq!(about(bruno).len(), 1, "Bruno's notice names him");

        let strings = MessagingStringsRegistry::new();
        let outcome = tick(&repos, &service, &strings).await.unwrap();
        assert_eq!(
            outcome.digests_sent, 3,
            "Alice, Bruno and the coach's own: {outcome:?}"
        );
        assert_eq!(outcome.errors, 0);

        let athlete_digests = digests_of(
            &service,
            coach,
            tenant,
            NotificationEvent::PersonaAthleteDigest,
        )
        .await;
        assert_eq!(athlete_digests.len(), 2, "one digest per athlete");
        let for_athlete = |name: &str| {
            athlete_digests
                .iter()
                .find(|d| {
                    event_params(d.data.as_ref()).and_then(|p| p.get("athlete_name"))
                        == Some(&json!(name))
                })
                .unwrap_or_else(|| panic!("no digest names {name}"))
        };
        let alice_digest = for_athlete("Alice");
        assert_eq!(returned(db, alice_digest).await, about(alice));
        assert_eq!(alice_digest.title, "Notification digest: Alice");
        assert!(
            alice_digest
                .body
                .starts_with("2 notification(s) about Alice"),
            "{}",
            alice_digest.body
        );
        let bruno_digest = for_athlete("Bruno");
        assert_eq!(returned(db, bruno_digest).await, about(bruno));
        assert_eq!(bruno_digest.title, "Notification digest: Bruno");

        let own_digests = digests_of(
            &service,
            coach,
            tenant,
            NotificationEvent::PersonaDailyDigest,
        )
        .await;
        assert_eq!(
            own_digests.len(),
            1,
            "the coach's own held row rolls up on its own"
        );
        assert_eq!(returned(db, &own_digests[0]).await, ids(&[own]));

        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            0,
            "every held notice is returned once"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // per_session
    // ════════════════════════════════════════════════════════════════

    /// The tick never sends a per-session digest; the athlete's next session
    /// does, returning exactly what was held, and a session with nothing held
    /// since sends nothing.
    #[tokio::test]
    async fn per_session_digest_waits_for_the_next_session() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, sink) = gated_service(&resources, POWER_P2_PER_SESSION);
        let (user, tenant) = seed_user(
            &resources,
            "digest_session@example.com",
            CoachingPersona::PowerAthlete,
            "fr",
            true,
        )
        .await;
        let strings = MessagingStringsRegistry::new();

        let held = [
            hold(&service, user, tenant, "activity_synced", PushTier::P3).await,
            hold(&service, user, tenant, "personal_record", PushTier::P3).await,
        ];
        let outcome = tick(&repos, &service, &strings).await.unwrap();
        assert_eq!(
            outcome.digests_sent, 0,
            "the clock never sends per_session: {outcome:?}"
        );
        assert!(
            sink.seen().is_empty(),
            "nothing went out before the session"
        );

        let sent = session_landed(&repos, &service, &strings, user, tenant)
            .await
            .unwrap();
        assert_eq!(sent, 1);
        assert_eq!(sink.seen(), ["persona_session_digest"]);
        let digests = digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaSessionDigest,
        )
        .await;
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].title, "Ton récap de notifications après séance");
        assert!(
            digests[0]
                .body
                .starts_with("2 notification(s) mises de côté depuis ta dernière séance"),
            "{}",
            digests[0].body
        );
        assert_eq!(returned(db, &digests[0]).await, ids(&held));

        assert_eq!(
            session_landed(&repos, &service, &strings, user, tenant)
                .await
                .unwrap(),
            0,
            "a session with nothing held since the last digest sends nothing"
        );
    }

    /// A session landing in the activity cache is what sends the digest: the
    /// first session lands, a re-served copy of it does not, a newer one does.
    #[tokio::test]
    async fn a_session_landing_in_the_cache_sends_the_per_session_digest() {
        let resources = create_test_server_resources().await.unwrap();
        let db = &resources.agent.database;
        resources
            .fitness
            .persona_contract_registry
            .apply_overlay(POWER_P2_PER_SESSION)
            .unwrap();
        let (user, tenant) = seed_user(
            &resources,
            "digest_landing@example.com",
            CoachingPersona::PowerAthlete,
            "en",
            true,
        )
        .await;
        let service = resources
            .common
            .notification_service
            .clone()
            .expect("the server wires a notification service");
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let ride = |id: &str, start: DateTime<Utc>| {
            ActivityBuilder::new(id, "Morning ride", SportType::Ride, start, 3_600, "strava")
                .build()
        };
        let first_session = ride("ride-1", Utc::now() - Duration::hours(3));

        let first_held = [
            hold(&service, user, tenant, "training_load_alert", PushTier::P3).await,
            hold(&service, user, tenant, "fitness_improvement", PushTier::P3).await,
        ];
        assert!(digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaSessionDigest
        )
        .await
        .is_empty());

        write_through_served_window(
            &runtime,
            user,
            &tenant,
            "strava",
            &ActivityQueryParams::default(),
            slice::from_ref(&first_session),
        )
        .await;
        let digests = wait_for_session_digests(&service, user, tenant, 1).await;
        assert_eq!(digests[0].title, "Your post-session notification digest");
        assert_eq!(returned(db, &digests[0]).await, ids(&first_held));
        let first_digest = digests[0].id;

        // The same session served again is already cached: nothing lands.
        // Nor does the same ride recorded by a second provider, whose cache
        // was empty until now.
        let between = hold(&service, user, tenant, "milestone_reached", PushTier::P3).await;
        write_through_served_window(
            &runtime,
            user,
            &tenant,
            "strava",
            &ActivityQueryParams::default(),
            slice::from_ref(&first_session),
        )
        .await;
        let garmin_copy = ActivityBuilder::new(
            "garmin-ride-1",
            "Morning ride",
            SportType::Ride,
            first_session.start_date(),
            3_600,
            "garmin",
        )
        .build();
        write_through_served_window(
            &runtime,
            user,
            &tenant,
            "garmin",
            &ActivityQueryParams::default(),
            slice::from_ref(&garmin_copy),
        )
        .await;
        sleep(StdDuration::from_millis(300)).await;
        assert_eq!(
            digests_of(
                &service,
                user,
                tenant,
                NotificationEvent::PersonaSessionDigest
            )
            .await
            .len(),
            1,
            "a session already in the cache does not land twice"
        );

        let after = hold(&service, user, tenant, "training_load_alert", PushTier::P3).await;
        let second_session = ride("ride-2", Utc::now() - Duration::minutes(10));
        write_through_served_window(
            &runtime,
            user,
            &tenant,
            "strava",
            &ActivityQueryParams::default(),
            &[second_session, first_session],
        )
        .await;
        let digests = wait_for_session_digests(&service, user, tenant, 2).await;
        let second = digests.iter().find(|d| d.id != first_digest).unwrap();
        assert_eq!(
            returned(db, second).await,
            ids(&[between, after]),
            "the next session returns everything held since the previous one"
        );
    }

    /// Poll until `user` holds `count` per-session digests — the landing
    /// spawns the digest off the fetch's path.
    async fn wait_for_session_digests(
        service: &NotificationService,
        user: Uuid,
        tenant: TenantId,
        count: usize,
    ) -> Vec<Notification> {
        let deadline = Instant::now() + StdDuration::from_secs(10);
        loop {
            let digests = digests_of(
                service,
                user,
                tenant,
                NotificationEvent::PersonaSessionDigest,
            )
            .await;
            if digests.len() >= count {
                assert_eq!(digests.len(), count, "one digest per landed session");
                return digests;
            }
            assert!(
                Instant::now() < deadline,
                "{} of {count} per-session digests arrived",
                digests.len()
            );
            sleep(StdDuration::from_millis(25)).await;
        }
    }

    // ════════════════════════════════════════════════════════════════
    // what a digest returned stays returned
    // ════════════════════════════════════════════════════════════════

    /// An athlete deleting a digest from their feed does not make its rows
    /// due again: the next digest returns only what was held since.
    #[tokio::test]
    async fn deleting_a_digest_does_not_return_its_rows_again() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, _sink) = gated_service(&resources, ENTHUSIAST_P1_DAILY);
        let (user, tenant) = seed_user(
            &resources,
            "digest_deleted@example.com",
            CoachingPersona::Enthusiast,
            "en",
            true,
        )
        .await;
        let strings = MessagingStringsRegistry::new();

        let held = [
            hold(&service, user, tenant, "training_load_alert", PushTier::P2).await,
            hold(&service, user, tenant, "personal_record", PushTier::P3).await,
        ];
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        let first = digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaDailyDigest,
        )
        .await
        .remove(0);
        assert_eq!(returned(db, &first).await, ids(&held));

        assert!(service
            .delete_notification(user, comm(tenant), first.id)
            .await
            .unwrap());
        backdate(db, &first, Utc::now() - Duration::days(1)).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            0,
            "the deleted digest's rows were returned; nothing else is held"
        );

        let next = hold(&service, user, tenant, "low_recovery_score", PushTier::P2).await;
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        let digests = digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaDailyDigest,
        )
        .await;
        assert_eq!(digests.len(), 1, "only the new digest is in the feed");
        assert_eq!(returned(db, &digests[0]).await, ids(&[next]));
        assert_eq!(item_count(&digests[0]), Some(1));
    }

    /// Two sessions landing at once send one digest, not two copies of it.
    #[tokio::test]
    async fn concurrent_session_landings_return_each_row_once() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, sink) = gated_service(&resources, POWER_P2_PER_SESSION);
        let (user, tenant) = seed_user(
            &resources,
            "digest_race@example.com",
            CoachingPersona::PowerAthlete,
            "en",
            true,
        )
        .await;
        let strings = MessagingStringsRegistry::new();
        let held = [
            hold(&service, user, tenant, "activity_synced", PushTier::P3).await,
            hold(&service, user, tenant, "personal_record", PushTier::P3).await,
            hold(&service, user, tenant, "milestone_reached", PushTier::P3).await,
        ];

        let (first, second) = tokio::join!(
            session_landed(&repos, &service, &strings, user, tenant),
            session_landed(&repos, &service, &strings, user, tenant),
        );
        assert_eq!(first.unwrap() + second.unwrap(), 1, "one digest in all");
        assert_eq!(sink.seen(), ["persona_session_digest"]);
        let digests = digests_of(
            &service,
            user,
            tenant,
            NotificationEvent::PersonaSessionDigest,
        )
        .await;
        assert_eq!(digests.len(), 1);
        assert_eq!(returned(db, &digests[0]).await, ids(&held));
        assert_eq!(item_count(&digests[0]), Some(3));
    }

    /// A digest the pipeline suppressed stores nothing, so it returns
    /// nothing: its rows stay held and the next digest returns them.
    #[tokio::test]
    async fn a_suppressed_digest_leaves_its_rows_for_the_next_one() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let db = &resources.agent.database;
        let (service, sink) = gated_service(&resources, CASUAL_P0_WEEKLY);
        let (user, tenant) = seed_user(
            &resources,
            "digest_suppressed@example.com",
            CoachingPersona::Casual,
            "en",
            true,
        )
        .await;
        let strings = MessagingStringsRegistry::new();
        let held = [
            hold(&service, user, tenant, "training_load_alert", PushTier::P2).await,
            hold(&service, user, tenant, "coach_message", PushTier::P1).await,
        ];
        let system_enabled = |enabled: bool| UpsertNotificationPreferenceParams {
            user_id: user,
            tenant_id: comm(tenant),
            category: NotificationCategory::System.as_str().to_owned(),
            enabled,
            sub_preferences: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            timezone: None,
            max_per_day: None,
        };

        service
            .upsert_notification_preference(&system_enabled(false))
            .await
            .unwrap();
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            0,
            "a suppressed digest is not sent"
        );
        assert!(sink.seen().is_empty());
        assert_eq!(
            repos
                .persona_digest_returns
                .last_persona_digest_at(user, tenant)
                .await
                .unwrap(),
            None,
            "the suppressed digest released its claim"
        );

        service
            .upsert_notification_preference(&system_enabled(true))
            .await
            .unwrap();
        assert_eq!(
            tick(&repos, &service, &strings).await.unwrap().digests_sent,
            1
        );
        let digests = digests_of(&service, user, tenant, NotificationEvent::PersonaDigest).await;
        assert_eq!(digests.len(), 1);
        assert_eq!(returned(db, &digests[0]).await, ids(&held));
    }

    // ════════════════════════════════════════════════════════════════
    // unarmed
    // ════════════════════════════════════════════════════════════════

    /// A user whose policy is not armed produces no digest of any cadence,
    /// even with gated-looking rows present — the digest reads the same
    /// arming flag as the dispatch gate.
    #[tokio::test]
    async fn unarmed_user_gets_no_digest() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);
        let (service, sink) = gated_service(&resources, CASUAL_P0_WEEKLY);
        let (user, tenant) = seed_user(
            &resources,
            "digest_unarmed@example.com",
            CoachingPersona::Casual,
            "en",
            false,
        )
        .await;

        // Persist a row that carries the gated marker (as if written while
        // armed); the unarmed sweep must still skip the user.
        service
            .create_notification(&CreateNotificationParams {
                user_id: user,
                tenant_id: comm(tenant),
                category: NotificationCategory::Training,
                notification_type: "training_load_alert".to_owned(),
                title: "held".to_owned(),
                body: "held".to_owned(),
                data: Some(json!({ "persona_gated": true })),
                image_url: None,
                actions: None,
            })
            .await
            .unwrap();

        let strings = MessagingStringsRegistry::new();
        let outcome = tick(&repos, &service, &strings).await.unwrap();
        assert_eq!(outcome.digests_sent, 0, "unarmed ⇒ no digest: {outcome:?}");
        assert_eq!(outcome.users_eligible, 0);
        assert_eq!(
            session_landed(&repos, &service, &strings, user, tenant)
                .await
                .unwrap(),
            0
        );
        assert!(sink.seen().is_empty());
        assert!(
            digests_of(&service, user, tenant, NotificationEvent::PersonaDigest)
                .await
                .is_empty()
        );
    }
}
