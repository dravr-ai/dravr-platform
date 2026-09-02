// ABOUTME: registre#7 — the weekly digest returns the pushes the persona policy withheld
// ABOUTME: Drives the scheduler tick: one localized digest per armed user, idempotent on re-tick

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona digest scheduler end-to-end.
//!
//! Two armed casual athletes (one French, one English) accumulate
//! persona-gated notifications; one scheduler tick sends each of them exactly
//! one `persona_digest` System notification in their own locale carrying the
//! item count, and an immediate second tick sends nothing because the digest
//! window is derived from the previous digest row.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "client-notifications")]
mod digest_tests {
    use std::sync::Arc;

    use serde_json::json;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources, create_test_tenant};
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
    use pierre_core::feature_flags::FeatureKey;
    use pierre_core::models::CoachingPersona;
    use pierre_database::backends::factory::Database;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::models::{
        CreateNotificationParams, Notification, NotificationCategory,
    };
    use pierre_notifications::{
        DispatchOutcome, DispatchRequest, NotificationService, PersonaPolicyGate, PushTier,
        TenantId as CommTenantId,
    };
    use pierre_services::notification_digest_scheduler::{tick, PERSONA_DIGEST_TYPE};
    use pierre_services::persona_notification_policy_gate::PersonaNotificationPolicyGate;

    const CASUAL_P0_WEEKLY: &str = r"
version: 1
personas:
  casual:
    notification:
      tier_floor: P0
      digest: weekly
";

    fn notification_service(db: &Database) -> NotificationService {
        match db {
            Database::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
        }
    }

    /// Seed one armed casual user in the given locale; returns their id and
    /// commere tenant id.
    async fn seed_armed_user(
        resources: &ServerContext,
        email: &str,
        locale: &str,
    ) -> (Uuid, CommTenantId) {
        let (user, _token) = create_test_tenant(resources, email).await.unwrap();
        let repos = &resources.common.repos;
        repos
            .users
            .set_coaching_persona(user.id, CoachingPersona::Casual)
            .await
            .unwrap();
        repos.users.update_locale(user.id, locale).await.unwrap();
        repos
            .feature_flags
            .set_user_override(user.id, FeatureKey::PersonaNotificationPolicy, true, None)
            .await
            .unwrap();
        let tenant = repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;
        (user.id, CommTenantId(tenant.as_uuid()))
    }

    /// Dispatch a P2 event through the armed gate so it lands persisted with
    /// the `persona_gated` marker — the exact rows the digest collects.
    async fn seed_gated_rows(
        service: &NotificationService,
        user_id: Uuid,
        tenant: CommTenantId,
        types: &[&str],
    ) {
        for notification_type in types {
            let outcome = service
                .dispatch_with_tier(
                    &DispatchRequest {
                        user_id,
                        tenant_id: tenant,
                        category: NotificationCategory::Training,
                        notification_type: (*notification_type).to_owned(),
                        title: "Training load elevated".to_owned(),
                        body: "held for the digest".to_owned(),
                        data: Some(json!({ "screen": "recovery" })),
                        image_url: None,
                        actions: None,
                        bypass_frequency_cap: false,
                    },
                    PushTier::P2,
                )
                .await
                .unwrap();
            assert!(
                matches!(outcome, DispatchOutcome::PersistedNoDevices { .. }),
                "seeding requires the armed gate to hold the row: {outcome:?}"
            );
        }
    }

    async fn digest_rows(
        service: &NotificationService,
        user_id: Uuid,
        tenant: CommTenantId,
    ) -> Vec<Notification> {
        let (rows, _, _) = service
            .list_notifications(
                user_id,
                tenant,
                100,
                0,
                Some(NotificationCategory::System.as_str()),
                false,
            )
            .await
            .unwrap();
        rows.into_iter()
            .filter(|n| n.notification_type == PERSONA_DIGEST_TYPE)
            .collect()
    }

    /// One tick sends one localized digest per armed weekly user; a second
    /// immediate tick sends nothing because no gated row postdates the digest.
    #[tokio::test]
    async fn one_localized_digest_per_user_then_silence() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);

        let (fr_user, fr_tenant) = seed_armed_user(&resources, "digest_fr@example.com", "fr").await;
        let (en_user, en_tenant) = seed_armed_user(&resources, "digest_en@example.com", "en").await;

        let registry = Arc::new(PersonaContractRegistry::new());
        registry.apply_overlay(CASUAL_P0_WEEKLY).unwrap();
        let gate: Arc<dyn PersonaPolicyGate> = Arc::new(PersonaNotificationPolicyGate::new(
            Arc::clone(&repos),
            Arc::clone(&registry),
        ));

        let service = notification_service(&resources.coach.database).with_policy_gate(Arc::new(
            PersonaNotificationPolicyGate::new(Arc::clone(&repos), registry),
        ));

        seed_gated_rows(
            &service,
            fr_user,
            fr_tenant,
            &["training_load_alert", "low_recovery_score"],
        )
        .await;
        seed_gated_rows(
            &service,
            en_user,
            en_tenant,
            &["training_load_alert", "low_recovery_score", "coach_message"],
        )
        .await;

        let strings = MessagingStringsRegistry::new();
        let outcome = tick(&repos, &gate, &service, &strings).await.unwrap();
        assert_eq!(
            outcome.digests_sent, 2,
            "one digest per armed weekly user: {outcome:?}"
        );
        assert_eq!(outcome.errors, 0, "no per-user errors: {outcome:?}");

        // French athlete: exact localized title, 2 held items in the body.
        let fr_digests = digest_rows(&service, fr_user, fr_tenant).await;
        assert_eq!(fr_digests.len(), 1, "exactly one digest for the fr user");
        assert_eq!(fr_digests[0].title, "Ton récap hebdo de notifications");
        assert!(
            fr_digests[0].body.starts_with("2 notification(s)"),
            "fr body carries the item count: {}",
            fr_digests[0].body
        );
        assert_eq!(
            fr_digests[0]
                .data
                .as_ref()
                .and_then(|d| d.get("item_count")),
            Some(&json!(2))
        );

        // English athlete: exact localized title, 3 held items in the body.
        let en_digests = digest_rows(&service, en_user, en_tenant).await;
        assert_eq!(en_digests.len(), 1, "exactly one digest for the en user");
        assert_eq!(en_digests[0].title, "Your weekly notification digest");
        assert!(
            en_digests[0].body.starts_with("3 notification(s)"),
            "en body carries the item count: {}",
            en_digests[0].body
        );

        // An immediate second tick finds no gated row newer than the digest
        // and sends nothing — restart/retry safety.
        let second = tick(&repos, &gate, &service, &strings).await.unwrap();
        assert_eq!(
            second.digests_sent, 0,
            "no new gated rows ⇒ no second digest: {second:?}"
        );
        assert_eq!(digest_rows(&service, fr_user, fr_tenant).await.len(), 1);
        assert_eq!(digest_rows(&service, en_user, en_tenant).await.len(), 1);
    }

    /// A user whose policy is not armed produces no digest even with gated-
    /// looking rows present — the scheduler reads the same arming flag as the
    /// dispatch gate.
    #[tokio::test]
    async fn unarmed_user_gets_no_digest() {
        let resources = create_test_server_resources().await.unwrap();
        let repos = Arc::clone(&resources.common.repos);

        let (user, _token) = create_test_tenant(&resources, "digest_unarmed@example.com")
            .await
            .unwrap();
        repos
            .users
            .set_coaching_persona(user.id, CoachingPersona::Casual)
            .await
            .unwrap();
        let tenant = repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .id;
        let tenant = CommTenantId(tenant.as_uuid());

        let registry = Arc::new(PersonaContractRegistry::new());
        registry.apply_overlay(CASUAL_P0_WEEKLY).unwrap();
        let gate: Arc<dyn PersonaPolicyGate> = Arc::new(PersonaNotificationPolicyGate::new(
            Arc::clone(&repos),
            registry,
        ));
        let service = notification_service(&resources.coach.database);

        // Persist a row that carries the gated marker (as if written while
        // armed); the unarmed sweep must still skip the user.
        service
            .create_notification(&CreateNotificationParams {
                user_id: user.id,
                tenant_id: tenant,
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
        let outcome = tick(&repos, &gate, &service, &strings).await.unwrap();
        assert_eq!(outcome.digests_sent, 0, "unarmed ⇒ no digest: {outcome:?}");
        assert!(digest_rows(&service, user.id, tenant).await.is_empty());
    }
}
