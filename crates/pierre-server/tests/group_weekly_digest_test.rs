// ABOUTME: The group weekly digest reaches each manager in their own language, with per-member context
// ABOUTME: Covers the digest parameters, the rendered French and English text, and the stored row's re-render
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-notifications")]
mod group_weekly_digest_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_core::models::groups::{
        GroupAggregateStats, GroupTrend, MemberFitnessSnapshot, OvertrainingRiskLevel,
    };
    use pierre_database::backends::factory::Database;
    use pierre_groups::GroupService;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::models::NotificationCategory;
    use pierre_notifications::{
        EventDispatch, NotificationEvent, NotificationService, PushTier,
        TenantId as CommereTenantId,
    };
    use pierre_routes_groups::group_digest_scheduler::digest_params;
    use pierre_routes_groups::NotificationRoutes;
    use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
    use pierre_services::notification_text::NotificationTextRenderer;
    use serde_json::{Map, Value};
    use uuid::Uuid;

    use crate::common::{create_test_server_resources, create_test_tenant};
    use crate::helpers::axum_test::AxumTestRequest;

    fn snapshot(
        name: &str,
        km: f64,
        prev_km: Option<f64>,
        form: Option<(f64, f64)>,
        idle_days: Option<i32>,
    ) -> MemberFitnessSnapshot {
        MemberFitnessSnapshot {
            user_id: Uuid::new_v4(),
            display_name: name.to_owned(),
            ctl: form.map(|(ctl, _)| ctl),
            atl: None,
            tsb: form.map(|(_, tsb)| tsb),
            weekly_volume_km: km,
            previous_week_volume_km: prev_km,
            weekly_activity_count: 3,
            weekly_duration_seconds: 3600,
            primary_sport: None,
            vdot: None,
            overtraining_risk: OvertrainingRiskLevel::Low,
            days_since_last_activity: idle_days,
            last_activity_per_provider: HashMap::new(),
            recent_activities: Vec::new(),
            needs_reauth_providers: Vec::new(),
            served_stale: false,
            timezone: None,
            computed_at: Utc::now(),
        }
    }

    /// Three members: one fresh, one deep in fatigue, one idle for ten days.
    fn week() -> (GroupAggregateStats, Vec<MemberFitnessSnapshot>) {
        let snapshots = vec![
            snapshot("Phil", 180.5, Some(150.0), Some((100.0, 12.0)), Some(1)),
            snapshot("Marie", 242.0, Some(200.0), Some((100.0, -40.0)), Some(0)),
            snapshot("Luc", 0.0, None, None, Some(10)),
        ];
        let stats = GroupAggregateStats {
            total_members: 3,
            active_members: 2,
            avg_weekly_volume_km: 140.833,
            avg_ctl: Some(100.0),
            flagged_members: 0,
            weekly_trend: GroupTrend::Improving,
        };
        (stats, snapshots)
    }

    fn params() -> Value {
        let (stats, snapshots) = week();
        let flags = GroupService::compute_health_flags(&snapshots);
        digest_params("Les Rouleurs", &stats, &snapshots, &flags)
    }

    fn render(locale: &str, params: &Value) -> (String, String) {
        let strings = MessagingStringsRegistry::new();
        let renderer = NotificationTextRenderer::new(&strings, locale);
        let empty = Map::new();
        let params = params.as_object().unwrap_or(&empty);
        (
            renderer.title(NotificationEvent::GroupWeeklyDigest, params),
            renderer.body(NotificationEvent::GroupWeeklyDigest, params),
        )
    }

    const FRENCH_BODY: &str = "\
2/3 membres actifs cette semaine, 140,8 km en moyenne par membre.
Volume du groupe en hausse par rapport à la semaine dernière — veille à ce que la récupération suive.

Volume de la semaine :
• Marie : 242,0 km (semaine précédente : 200,0 km)
• Phil : 180,5 km (semaine précédente : 150,0 km)
• Luc : 0,0 km

En forme :
• Phil : forme fraîche (+12 % de sa charge chronique)

À surveiller :
• Marie : forme à -40 % de sa charge chronique, fatigue profonde
• Luc : aucune activité depuis 10 jours";

    /// Numbers stay numbers and states stay codes in the stored parameters,
    /// so nothing in them is English.
    #[test]
    fn digest_params_carry_codes_and_numbers_not_sentences() {
        let params = params();
        assert_eq!(params["group_name"], "Les Rouleurs");
        assert_eq!(params["active_members"], 2);
        assert_eq!(params["total_members"], 3);
        assert_eq!(params["trend"], "improving");

        let members = params["members"].as_array().unwrap();
        let names: Vec<&str> = members
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Marie", "Phil", "Luc"], "sorted by weekly volume");
        assert_eq!(members[2]["prev_km"], Value::Null);

        let highlights = params["highlights"].as_array().unwrap();
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0]["name"], "Phil");

        let concerns = params["concerns"].as_array().unwrap();
        let codes: Vec<(&str, &str)> = concerns
            .iter()
            .map(|c| (c["name"].as_str().unwrap(), c["code"].as_str().unwrap()))
            .collect();
        assert_eq!(codes, [("Marie", "deep_fatigue"), ("Luc", "inactive")]);
        assert_eq!(concerns[1]["value"], 10);
    }

    /// The Telegram digest that motivated this read, in English, to a French
    /// group, as one line with no names in it.
    #[test]
    fn a_french_manager_reads_the_digest_in_french_with_every_member() {
        let (title, body) = render("fr", &params());
        assert_eq!(title, "Récap hebdo : Les Rouleurs");
        assert_eq!(body, FRENCH_BODY);
    }

    #[test]
    fn an_english_manager_reads_the_same_parameters_in_english() {
        let (title, body) = render("en", &params());
        assert_eq!(title, "Weekly recap: Les Rouleurs");
        assert!(
            body.starts_with("2/3 members active this week, averaging 140.8 km each."),
            "{body}"
        );
        assert!(
            body.contains("• Marie: 242.0 km (previous week: 200.0 km)"),
            "{body}"
        );
        assert!(
            body.contains("• Phil: fresh form (+12% of chronic load)"),
            "{body}"
        );
        assert!(body.contains("• Luc: no activity for 10 days"), "{body}");
    }

    /// With nobody fresh and nobody flagged the digest says so, instead of
    /// ending on the roster as if something were missing.
    #[test]
    fn a_quiet_week_closes_with_the_all_clear() {
        let snapshots = vec![snapshot("Phil", 40.0, Some(40.0), None, Some(2))];
        let stats = GroupAggregateStats {
            total_members: 1,
            active_members: 1,
            avg_weekly_volume_km: 40.0,
            avg_ctl: None,
            flagged_members: 0,
            weekly_trend: GroupTrend::Stable,
        };
        let params = digest_params("Solo", &stats, &snapshots, &[]);
        let (_, body) = render("fr", &params);
        assert!(
            body.ends_with("\n\nRien à signaler côté forme et assiduité cette semaine."),
            "{body}"
        );
        assert!(body.contains("Volume du groupe stable"), "{body}");
    }

    fn notification_service(resources: &ServerContext) -> NotificationService {
        let service = match &*resources.agent.database {
            Database::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
        };
        service.with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.mcp.messaging_strings_registry),
        )))
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

    /// Dispatched the way the scheduler dispatches it, the digest is stored
    /// in the manager's language and re-rendered when that language changes.
    #[tokio::test]
    async fn the_stored_digest_follows_the_managers_language() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, token) = create_test_tenant(&resources, "group_digest@example.com")
            .await
            .unwrap();
        let token = format!("Bearer {token}");
        let tenants = resources
            .common
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant = CommereTenantId(tenants.first().unwrap().id.as_uuid());
        let service = notification_service(&resources);
        let router = NotificationRoutes::routes(Arc::clone(&resources));

        service
            .dispatch_event(
                &EventDispatch {
                    user_id: user.id,
                    tenant_id: tenant,
                    category: NotificationCategory::Coach,
                    event: NotificationEvent::GroupWeeklyDigest,
                    params: params(),
                    route: Value::Null,
                    actions: None,
                    bypass_frequency_cap: false,
                },
                PushTier::P3,
            )
            .await
            .unwrap();

        // The test user has the default locale, French.
        let rows = feed(&router, &token).await;
        let row = rows.first().expect("the digest is stored");
        assert_eq!(row["notification_type"], "group_weekly_digest");
        assert_eq!(row["title"], "Récap hebdo : Les Rouleurs");
        assert_eq!(row["body"], FRENCH_BODY);

        resources
            .common
            .repos
            .users
            .update_locale(user.id, "en")
            .await
            .unwrap();
        let rows = feed(&router, &token).await;
        let row = rows.first().expect("the same row");
        assert_eq!(row["title"], "Weekly recap: Les Rouleurs");
        assert!(
            row["body"]
                .as_str()
                .unwrap()
                .contains("• Marie: form at -40% of chronic load, deepest fatigue band"),
            "{}",
            row["body"]
        );
    }
}
