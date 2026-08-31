// ABOUTME: The notification dispatcher's third sink — an athlete who lives in chat finally gets told
// ABOUTME: Asserts the fan-out fires only on accepted notifications, per link, in that link's locale

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#63: the notification dispatcher had no messaging sink.
//!
//! `dravr-commere` persists the notification row and pushes to Expo devices.
//! Those are its only two outlets, so an athlete who talks to Dravr on
//! Telegram or Slack and never installed the mobile app received nothing,
//! whatever the category.
//!
//! Two halves are asserted here:
//!
//! - the fan-out contract: [`NotificationService::dispatch`] runs a sink for a
//!   notification the pipeline accepted and does *not* run it for one the
//!   pipeline suppressed, so the sink can never route around a preference;
//! - the outbound resolution: `send_to_linked_channels` walks every link the
//!   user has, in that link's own locale, which is what makes the sink's
//!   registry render locale-correct per channel.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(all(feature = "client-notifications", feature = "client-messaging"))]
mod sink_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use async_trait::async_trait;
    use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
    use pierre_core::models::messaging::ChannelType;
    use pierre_database::backends::factory::Database;
    use pierre_database::backends::CreateChannelLinkParams;
    use pierre_notifications::models::{NotificationCategory, UpsertNotificationPreferenceParams};
    use pierre_notifications::{
        DispatchOutcome, DispatchRequest, NotificationChannelSink, NotificationService,
        SuppressionReason, TenantId as CommTenantId,
    };
    use pierre_services::messaging_broadcast::{resolve_linked_targets, LinkedChannelTarget};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    /// A sink that records what it was handed, so "the sink ran" is asserted by
    /// the notification's own title and body rather than by a log line.
    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<(Uuid, String, String)>>,
    }

    #[async_trait]
    impl NotificationChannelSink for RecordingSink {
        async fn deliver(&self, request: &DispatchRequest) {
            self.seen.lock().unwrap().push((
                request.user_id,
                request.title.clone(),
                request.body.clone(),
            ));
        }
    }

    fn request(
        user_id: Uuid,
        tenant: CommTenantId,
        category: NotificationCategory,
    ) -> DispatchRequest {
        DispatchRequest {
            user_id,
            tenant_id: tenant,
            category,
            notification_type: "coach_followup_due".to_owned(),
            title: "Your coach has a followup for you".to_owned(),
            body: "How did the tempo run go?".to_owned(),
            data: None,
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        }
    }

    /// The notification service on whichever backend the test database is —
    /// the same mapping the server performs at boot.
    fn notification_service(db: &Database) -> NotificationService {
        match db {
            Database::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
        }
    }

    #[tokio::test]
    async fn accepted_notification_reaches_the_channel_sink() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "sink_accepted@example.com")
            .await
            .unwrap();
        let tenant = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;

        let sink = Arc::new(RecordingSink::default());
        let service = notification_service(&resources.coach.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>);

        let outcome = service
            .dispatch(&request(
                user.id,
                CommTenantId(tenant.as_uuid()),
                NotificationCategory::Coach,
            ))
            .await
            .unwrap();
        assert!(
            matches!(outcome, DispatchOutcome::PersistedNoDevices { .. }),
            "no Expo device is registered, so the row is persisted with no push: {outcome:?}"
        );

        let seen = sink.seen.lock().unwrap().clone();
        assert_eq!(
            seen.len(),
            1,
            "an accepted notification runs the channel sink exactly once"
        );
        assert_eq!(seen[0].0, user.id);
        assert_eq!(seen[0].1, "Your coach has a followup for you");
        assert_eq!(seen[0].2, "How did the tempo run go?");
    }

    #[tokio::test]
    async fn suppressed_notification_never_reaches_the_channel_sink() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "sink_suppressed@example.com")
            .await
            .unwrap();
        let tenant = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;

        let sink = Arc::new(RecordingSink::default());
        let service = notification_service(&resources.coach.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>);

        // The athlete turned the category off. Messaging must respect that —
        // a sink that routed around it would be a second delivery ladder with
        // no preference of its own.
        service
            .upsert_notification_preference(&UpsertNotificationPreferenceParams {
                user_id: user.id,
                tenant_id: CommTenantId(tenant.as_uuid()),
                category: NotificationCategory::Coach.as_str().to_owned(),
                enabled: false,
                sub_preferences: None,
                quiet_hours_start: None,
                quiet_hours_end: None,
                timezone: None,
                max_per_day: None,
            })
            .await
            .unwrap();

        let outcome = service
            .dispatch(&request(
                user.id,
                CommTenantId(tenant.as_uuid()),
                NotificationCategory::Coach,
            ))
            .await
            .unwrap();
        assert!(
            matches!(
                outcome,
                DispatchOutcome::Suppressed(SuppressionReason::CategoryDisabled)
            ),
            "the disabled category suppresses the notification: {outcome:?}"
        );
        assert_eq!(
            sink.seen.lock().unwrap().len(),
            0,
            "a suppressed notification must not reach any platform sink"
        );
    }

    /// The outbound half: every link the user holds is resolved once, with its
    /// own locale, which is what lets the sink render one registry key into two
    /// languages for one athlete.
    ///
    /// Stops at the resolution rather than the send: the adapters post to
    /// hardcoded channel hosts, so a test that went one step further would be
    /// asserting the network.
    #[tokio::test]
    async fn linked_channels_resolve_once_each_in_their_own_locale() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "sink_links@example.com")
            .await
            .unwrap();
        let tenant = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;
        let messaging = resources.common.repos.messaging.as_ref();

        for channel in ["telegram", "slack"] {
            let link_id = Uuid::new_v4().to_string();
            messaging
                .create_channel_link(&CreateChannelLinkParams {
                    id: &link_id,
                    tenant_id: tenant,
                    user_id: &user.id.to_string(),
                    channel_type: channel,
                    channel_user_id: &format!("{channel}-user"),
                    display_name: Some("Test Athlete"),
                })
                .await
                .unwrap();
        }
        // One link speaks English; the other keeps the default locale.
        messaging
            .set_channel_link_locale(tenant, &user.id.to_string(), "telegram", Some("en"))
            .await
            .unwrap();

        let mut targets = resolve_linked_targets(messaging, tenant, user.id).await;
        targets.sort_by(|a, b| a.recipient_id.cmp(&b.recipient_id));

        assert_eq!(
            targets,
            vec![
                LinkedChannelTarget {
                    channel_type: ChannelType::Slack,
                    recipient_id: "slack-user".to_owned(),
                    locale: DEFAULT_LOCALE.to_owned(),
                },
                LinkedChannelTarget {
                    channel_type: ChannelType::Telegram,
                    recipient_id: "telegram-user".to_owned(),
                    locale: "en".to_owned(),
                },
            ],
            "both linked channels resolve, each carrying its own locale"
        );
    }

    /// An athlete with no linked channel resolves to nothing — that is an app-
    /// only user, not a failure.
    #[tokio::test]
    async fn an_unlinked_athlete_resolves_to_no_channels() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "sink_unlinked@example.com")
            .await
            .unwrap();
        let tenant = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;

        let targets =
            resolve_linked_targets(resources.common.repos.messaging.as_ref(), tenant, user.id)
                .await;
        assert!(
            targets.is_empty(),
            "an athlete with no channel link has nowhere to be messaged: {targets:?}"
        );
    }
}
