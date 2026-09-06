// ABOUTME: A notification row stores its event and parameters, and the feed renders it per locale
// ABOUTME: The same stored row reads French for a French athlete and English for an English one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names
)]

mod common;
mod helpers;

#[cfg(feature = "client-notifications")]
mod notification_event_locale_tests {
    use crate::common::{create_test_server_resources, create_test_tenant};
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_database::backends::factory::Database;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::events::{event_params, PARAMS_DATA_KEY};
    use pierre_notifications::{
        triggers as notification_triggers, NotificationEvent, NotificationService,
        TenantId as CommereTenantId,
    };
    use pierre_routes_groups::NotificationRoutes;
    use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// The notification service the server boots: the upstream pipeline plus
    /// the localizer that renders each event in the recipient's language.
    fn notification_service(resources: &ServerContext) -> NotificationService {
        let service = match &*resources.coach.database {
            Database::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
        };
        service.with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.mcp.messaging_strings_registry),
        )))
    }

    async fn tenant_of(resources: &ServerContext, user_id: Uuid) -> CommereTenantId {
        let tenants = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap();
        CommereTenantId(tenants.first().unwrap().id.as_uuid())
    }

    async fn feed(router: &axum::Router, token: &str) -> Vec<Value> {
        let response = AxumTestRequest::get("/api/notifications")
            .header("authorization", token)
            .send(router.clone())
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        body["data"].as_array().unwrap().clone()
    }

    /// The trigger records what happened, not a sentence in one language.
    ///
    /// Before the event vocabulary the row went into the database as
    /// `"Message from your coach"` / `"<coach> sent you a message"`, so every
    /// athlete on every surface — push included — read English and a later
    /// locale change repaired nothing.
    #[tokio::test]
    async fn coach_message_stores_its_event_and_parameters() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "event_params@example.com")
            .await
            .unwrap();
        let tenant = tenant_of(&resources, user.id).await;
        let service = Arc::new(notification_service(&resources));

        notification_triggers::trigger_coach_message(
            &service,
            user.id,
            tenant,
            "conv-77",
            "Coach Alice",
        );
        sleep(Duration::from_millis(300)).await;

        let (rows, _, _) = service
            .list_notifications(user.id, tenant, 10, 0, Some("coach"), false)
            .await
            .unwrap();
        let row = rows.first().expect("the trigger persists one row");

        assert_eq!(
            row.notification_type,
            NotificationEvent::CoachMessage.wire()
        );
        let params = event_params(row.data.as_ref()).expect("the row carries its parameters");
        assert_eq!(
            params.get("coach_name").and_then(Value::as_str),
            Some("Coach Alice")
        );

        // The deep-link payload rides alongside the parameters, untouched.
        let data = row.data.as_ref().unwrap();
        assert_eq!(data.get("screen").and_then(Value::as_str), Some("coach"));
        assert_eq!(data.get("id").and_then(Value::as_str), Some("conv-77"));
        assert!(data.get(PARAMS_DATA_KEY).is_some());

        // The stored text is the athlete's own language, because the push
        // carries it and a push cannot be re-read in another one. The test
        // user has the default locale, French.
        assert_eq!(row.title, "Message de ton agent");
        assert_eq!(row.body, "Coach Alice t'a envoyé un message");
        assert!(
            !row.title.contains("Message from your agent"),
            "the English sentence must not reach the database: {}",
            row.title
        );
    }

    /// One row, two readers, two languages — and the action button with them.
    #[tokio::test]
    async fn the_same_row_reads_french_then_english() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, token) = create_test_tenant(&resources, "event_locale@example.com")
            .await
            .unwrap();
        let token = format!("Bearer {token}");
        let tenant = tenant_of(&resources, user.id).await;
        let service = Arc::new(notification_service(&resources));
        let router = NotificationRoutes::routes(Arc::clone(&resources));

        notification_triggers::trigger_coach_message(
            &service,
            user.id,
            tenant,
            "conv-88",
            "Coach Alice",
        );
        sleep(Duration::from_millis(300)).await;

        let rows = feed(&router, &token).await;
        let row = rows.first().expect("the feed returns the row");
        assert_eq!(row["title"], "Message de ton agent");
        assert_eq!(row["body"], "Coach Alice t'a envoyé un message");
        assert_eq!(row["actions"][0]["id"], "reply");
        assert_eq!(row["actions"][0]["title"], "Répondre");
        let stored_id = row["id"].as_str().unwrap().to_owned();

        // Same row, different reader language: no re-dispatch, no new row.
        resources
            .common
            .repos
            .users
            .update_locale(user.id, "en")
            .await
            .unwrap();

        let rows = feed(&router, &token).await;
        let row = rows.first().expect("the feed returns the same row");
        assert_eq!(row["id"].as_str().unwrap(), stored_id);
        assert_eq!(row["title"], "Message from your agent");
        assert_eq!(row["body"], "Coach Alice sent you a message");
        assert_eq!(row["actions"][0]["title"], "Reply");
    }

    /// A row written before the event vocabulary carries no parameters, so the
    /// feed shows the text it was stored with rather than an empty sentence.
    #[tokio::test]
    async fn a_row_without_parameters_keeps_its_stored_text() {
        use pierre_notifications::models::{CreateNotificationParams, NotificationCategory};

        let resources = create_test_server_resources().await.unwrap();
        let (user, token) = create_test_tenant(&resources, "event_legacy@example.com")
            .await
            .unwrap();
        let token = format!("Bearer {token}");
        let tenant = tenant_of(&resources, user.id).await;
        let service = notification_service(&resources);
        let router = NotificationRoutes::routes(Arc::clone(&resources));

        service
            .create_notification(&CreateNotificationParams {
                user_id: user.id,
                tenant_id: tenant,
                category: NotificationCategory::Coach,
                notification_type: NotificationEvent::CoachMessage.wire().to_owned(),
                title: "Message from your coach".to_owned(),
                body: "Coach Alice sent you a message".to_owned(),
                data: Some(serde_json::json!({ "screen": "coach", "id": "conv-legacy" })),
                image_url: None,
                actions: None,
            })
            .await
            .unwrap();

        let rows = feed(&router, &token).await;
        let row = rows
            .first()
            .expect("the feed returns the pre-vocabulary row");
        assert_eq!(row["title"], "Message from your coach");
        assert_eq!(row["body"], "Coach Alice sent you a message");
    }

    /// Every trigger, not only the coach one, records its event and reads in
    /// the athlete's language.
    #[tokio::test]
    async fn every_trigger_renders_in_the_athletes_language() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, _token) = create_test_tenant(&resources, "event_all@example.com")
            .await
            .unwrap();
        let tenant = tenant_of(&resources, user.id).await;
        let service = Arc::new(notification_service(&resources));

        notification_triggers::trigger_activity_synced(
            &service, user.id, tenant, "act-1", "Course", "10,2 km", "52:14",
        );
        notification_triggers::trigger_training_load_alert(&service, user.id, tenant, 85.0);
        notification_triggers::trigger_low_recovery_score(&service, user.id, tenant, 32.0);
        notification_triggers::trigger_overtraining_warning(&service, user.id, tenant);
        notification_triggers::trigger_personal_record(
            &service, user.id, tenant, "act-2", "5 km", "22:14",
        );
        notification_triggers::trigger_milestone_reached(&service, user.id, tenant, "1 000", "km");
        notification_triggers::trigger_fitness_improvement(
            &service, user.id, tenant, "FTP", "265 W",
        );
        notification_triggers::trigger_plan_updated(&service, user.id, tenant, "Coach Alice");
        notification_triggers::trigger_coach_feedback(
            &service,
            user.id,
            tenant,
            "act-3",
            "Coach Alice",
            "sortie longue",
        );
        notification_triggers::trigger_sync_failure(
            &service,
            user.id,
            tenant,
            "Strava",
            "jeton expiré",
        );
        sleep(Duration::from_millis(600)).await;

        let (rows, _, _) = service
            .list_notifications(user.id, tenant, 50, 0, None, false)
            .await
            .unwrap();
        assert_eq!(rows.len(), 10, "every trigger persists exactly one row");

        let expected = [
            (
                "activity_synced",
                "Nouvelle activité synchronisée",
                "Course — 10,2 km en 52:14",
            ),
            (
                "training_load_alert",
                "Charge d'entraînement élevée",
                "Ta charge aiguë est à 85 — pense à une journée de récupération",
            ),
            (
                "low_recovery_score",
                "Score de récupération bas",
                "Ton score de récupération est de 32/100 — journée facile recommandée",
            ),
            (
                "overtraining_warning",
                "Risque de surentraînement détecté",
                "La tendance de ta charge d'entraînement indique une accumulation de fatigue",
            ),
            (
                "personal_record",
                "Nouveau record personnel !",
                "Nouveau record sur 5 km : 22:14",
            ),
            (
                "milestone_reached",
                "Palier atteint !",
                "Tu as cumulé 1 000 km cette année",
            ),
            (
                "fitness_improvement",
                "Progrès de forme détecté",
                "Ton FTP est passé à 265 W",
            ),
            (
                "plan_updated",
                "Plan d'entraînement mis à jour",
                "Coach Alice a mis à jour ton plan d'entraînement",
            ),
            (
                "coach_feedback",
                "Retour de ton agent",
                "Coach Alice a laissé une note sur ton sortie longue",
            ),
            (
                "sync_failure",
                "Échec de synchronisation Strava",
                "jeton expiré",
            ),
        ];

        for (wire, title, body) in expected {
            let row = rows
                .iter()
                .find(|row| row.notification_type == wire)
                .unwrap_or_else(|| panic!("no row for {wire}"));
            assert_eq!(row.title, title, "{wire} title");
            assert_eq!(row.body, body, "{wire} body");
            assert!(
                event_params(row.data.as_ref()).is_some(),
                "{wire} must store its parameters"
            );
        }
    }

    /// Every event the vocabulary declares round-trips through its wire form,
    /// so a stored `notification_type` always resolves back to its event.
    #[tokio::test]
    async fn every_event_round_trips_through_its_wire_form() {
        for event in [
            NotificationEvent::ActivitySynced,
            NotificationEvent::TrainingLoadAlert,
            NotificationEvent::LowRecoveryScore,
            NotificationEvent::OvertrainingWarning,
            NotificationEvent::PersonalRecord,
            NotificationEvent::MilestoneReached,
            NotificationEvent::FitnessImprovement,
            NotificationEvent::CoachMessage,
            NotificationEvent::PlanUpdated,
            NotificationEvent::CoachFeedback,
            NotificationEvent::SyncFailure,
            NotificationEvent::PersonaDigest,
        ] {
            assert_eq!(NotificationEvent::from_wire(event.wire()), Some(event));
        }
        assert_eq!(NotificationEvent::from_wire("not_an_event"), None);
    }
}
