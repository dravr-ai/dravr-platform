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
//!   registry render locale-correct per channel — including a link made
//!   through the deployment bot, which is stored under the bot's tenant rather
//!   than the athlete's (carnet#439), and is sent by that bot.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(all(feature = "client-notifications", feature = "client-messaging"))]
mod sink_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use async_trait::async_trait;
    use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
    use pierre_core::models::messaging::ChannelType;
    use pierre_core::transport::TransportPolicy;
    use pierre_database::backends::factory::{Database, DatabaseBackend};
    use pierre_database::backends::{CreateChannelLinkParams, UpsertChannelConfigParams};
    use pierre_notifications::models::{NotificationCategory, UpsertNotificationPreferenceParams};
    use pierre_notifications::{
        DispatchOutcome, DispatchRequest, EventDispatch, NotificationChannelSink,
        NotificationEvent, NotificationService, PushTier, SuppressionReason,
        TenantId as CommTenantId,
    };
    use pierre_services::messaging_broadcast::{
        resolve_linked_targets, resolve_target_sender, LinkedChannelTarget,
    };
    use pierre_services::notification_channel_sink::MessagingChannelSink;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio_util::task::TaskTracker;
    use uuid::Uuid;

    /// A sink that records what it was handed, so "the sink ran" is asserted by
    /// the notification's own title and body rather than by a log line. It
    /// stands for one linked channel.
    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<(Uuid, String, String)>>,
    }

    #[async_trait]
    impl NotificationChannelSink for RecordingSink {
        async fn deliver(&self, request: &DispatchRequest) -> usize {
            self.seen.lock().unwrap().push((
                request.user_id,
                request.title.clone(),
                request.body.clone(),
            ));
            1
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
            title: "Your agent has a followup for you".to_owned(),
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
        match db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
            }
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
        let service = notification_service(&resources.agent.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>);

        let outcome = service
            .dispatch_with_tier(
                &request(
                    user.id,
                    CommTenantId(tenant.as_uuid()),
                    NotificationCategory::Coach,
                ),
                PushTier::P1,
            )
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
        assert_eq!(seen[0].1, "Your agent has a followup for you");
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
        let service = notification_service(&resources.agent.database)
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
            .dispatch_with_tier(
                &request(
                    user.id,
                    CommTenantId(tenant.as_uuid()),
                    NotificationCategory::Coach,
                ),
                PushTier::P1,
            )
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

        let mut targets = resolve_linked_targets(messaging, user.id).await;
        targets.sort_by(|a, b| a.recipient_id.cmp(&b.recipient_id));

        assert_eq!(
            targets,
            vec![
                LinkedChannelTarget {
                    channel_type: ChannelType::Slack,
                    recipient_id: "slack-user".to_owned(),
                    locale: DEFAULT_LOCALE.to_owned(),
                    tenant_id: tenant,
                },
                LinkedChannelTarget {
                    channel_type: ChannelType::Telegram,
                    recipient_id: "telegram-user".to_owned(),
                    locale: "en".to_owned(),
                    tenant_id: tenant,
                },
            ],
            "both linked channels resolve, each carrying its own locale"
        );
    }

    /// `dispatch_event` reports how many linked channels the sink reached: one
    /// here, and zero for the production messaging sink when the athlete links
    /// no channel, or when their settings suppress the notification — so a
    /// persisted row alone never reads as the athlete having been told.
    #[tokio::test]
    async fn dispatch_event_reports_the_channels_it_reached() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "sink_reach@example.com")
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
        let event = EventDispatch {
            user_id: user.id,
            tenant_id: CommTenantId(tenant.as_uuid()),
            category: NotificationCategory::Coach,
            event: NotificationEvent::SeatReleaseWarning,
            params: json!({ "provider_name": "Strava", "idle_days": "12", "days_left": "3" }),
            route: json!({ "screen": "connections" }),
            actions: None,
            bypass_frequency_cap: false,
            transport_policy: TransportPolicy::AnyTransport,
        };

        let sink = Arc::new(RecordingSink::default());
        let linked = notification_service(&resources.agent.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>);
        let delivery = linked.dispatch_event(&event, PushTier::P1).await.unwrap();
        assert_eq!(delivery.channels, 1, "{delivery:?}");
        assert!(delivery.reached_outside_the_app());

        let production = notification_service(&resources.agent.database).with_channel_sink(
            Arc::new(MessagingChannelSink::new(
                Arc::clone(&resources.common.repos),
                Arc::clone(&resources.mcp.messaging_strings_registry),
            )),
        );
        let unlinked = production
            .dispatch_event(&event, PushTier::P1)
            .await
            .unwrap();
        assert!(
            matches!(unlinked.outcome, DispatchOutcome::PersistedNoDevices { .. }),
            "{unlinked:?}"
        );
        assert_eq!(unlinked.channels, 0, "no link, no channel reached");
        assert!(!unlinked.reached_outside_the_app());

        linked
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
        let suppressed = linked.dispatch_event(&event, PushTier::P1).await.unwrap();
        assert!(
            matches!(
                suppressed.outcome,
                DispatchOutcome::Suppressed(SuppressionReason::CategoryDisabled)
            ),
            "{suppressed:?}"
        );
        assert_eq!(suppressed.channels, 0);
        assert!(!suppressed.reached_outside_the_app());
        assert_eq!(
            sink.seen.lock().unwrap().len(),
            1,
            "the sink ran once, unsuppressed"
        );
    }

    /// carnet#439: the deployment bot is configured under the admin's tenant,
    /// and ingress stores every link it makes under that bot tenant, while a
    /// self-registered athlete lives in a personal tenant. A notification
    /// raised in the athlete's tenant must still reach that chat, through the
    /// bot that holds it — and never reach another user's chat on the same bot.
    ///
    /// Stops at the resolution and the bot's config, for the reason the test
    /// above gives: the adapter posts to the channel's real host.
    #[tokio::test]
    async fn a_bot_made_link_under_another_tenant_is_reached_through_that_bot() {
        let resources = create_test_server_resources().await.unwrap();
        let messaging = resources.common.repos.messaging.as_ref();
        let tenants = &resources.common.repos.tenants;

        let (bot_admin, _) = create_test_tenant(&resources, "sink_bot_admin@example.com")
            .await
            .unwrap();
        let bot_tenant = tenants.list_for_user(bot_admin.id).await.unwrap()[0].id;
        let (athlete, _) = create_test_tenant(&resources, "sink_bot_athlete@example.com")
            .await
            .unwrap();
        let athlete_tenant = tenants.list_for_user(athlete.id).await.unwrap()[0].id;
        let (other, _) = create_test_tenant(&resources, "sink_bot_other@example.com")
            .await
            .unwrap();
        assert_ne!(
            athlete_tenant, bot_tenant,
            "the athlete lives apart from the bot"
        );

        // The deployment's Telegram bot, under the admin's tenant only.
        messaging
            .upsert_channel_config(&UpsertChannelConfigParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: bot_tenant,
                channel_type: "telegram",
                api_key: None,
                api_secret: None,
                webhook_secret: Some("sink_bot_secret"),
                verify_token: None,
                account_id: None,
                phone_number: None,
                bot_token: Some("439:sink-deployment-bot"),
                is_active: true,
            })
            .await
            .unwrap();
        assert!(messaging
            .mark_channel_config_platform_scope(bot_tenant, "telegram")
            .await
            .unwrap());

        // Both chats were opened with that bot, so both links live under it.
        for (user_id, chat) in [(athlete.id, "tg-athlete-439"), (other.id, "tg-other-439")] {
            messaging
                .create_channel_link(&CreateChannelLinkParams {
                    id: &Uuid::new_v4().to_string(),
                    tenant_id: bot_tenant,
                    user_id: &user_id.to_string(),
                    channel_type: "telegram",
                    channel_user_id: chat,
                    display_name: None,
                })
                .await
                .unwrap();
        }
        messaging
            .set_channel_link_locale(bot_tenant, &athlete.id.to_string(), "telegram", Some("fr"))
            .await
            .unwrap();

        let targets = resolve_linked_targets(messaging, athlete.id).await;
        assert_eq!(
            targets,
            vec![LinkedChannelTarget {
                channel_type: ChannelType::Telegram,
                recipient_id: "tg-athlete-439".to_owned(),
                locale: "fr".to_owned(),
                tenant_id: bot_tenant,
            }],
            "the athlete's bot-made link is their one target, carrying the bot's tenant"
        );

        // The sender is the bot that holds the chat, resolved by the same call
        // `send_to_linked_channels` makes: its config under the link's tenant.
        let (_, sender) = resolve_target_sender(messaging, &targets[0])
            .await
            .expect("the bot that holds the chat sends to it");
        assert_eq!(sender.tenant_id, bot_tenant.to_string());
        assert_eq!(sender.bot_token.as_deref(), Some("439:sink-deployment-bot"));
        assert_eq!(sender.channel_type, ChannelType::Telegram);

        // Scoped to the athlete's own tenant, the same chat has no bot to send
        // it: that is the read this path used to make, and it reached nothing.
        let athlete_scoped = LinkedChannelTarget {
            tenant_id: athlete_tenant,
            ..targets[0].clone()
        };
        assert!(resolve_target_sender(messaging, &athlete_scoped)
            .await
            .is_none());

        // The other user on the same bot resolves to their own chat only.
        let other_targets = resolve_linked_targets(messaging, other.id).await;
        let recipients: Vec<&str> = other_targets
            .iter()
            .map(|t| t.recipient_id.as_str())
            .collect();
        assert_eq!(recipients, vec!["tg-other-439"]);
    }

    /// An athlete with no linked channel resolves to nothing — that is an app-
    /// only user, not a failure.
    #[tokio::test]
    async fn an_unlinked_athlete_resolves_to_no_channels() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "sink_unlinked@example.com")
            .await
            .unwrap();

        let targets =
            resolve_linked_targets(resources.common.repos.messaging.as_ref(), user.id).await;
        assert!(
            targets.is_empty(),
            "an athlete with no channel link has nowhere to be messaged: {targets:?}"
        );
    }
}
