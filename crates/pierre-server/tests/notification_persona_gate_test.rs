// ABOUTME: registre#7 — the persona NotificationPolicy finally gates dispatched pushes
// ABOUTME: Pins floor semantics end-to-end: armed casual P0 floor holds P1-P3, delivers P0

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona push-policy gating through the dispatch facade.
//!
//! The persona contracts promised a notification cadence per persona while
//! nothing consumed the parsed `NotificationPolicy` (registre#7). These tests
//! assert the consumption: an **armed** casual user's P0 floor persists a
//! P1/P2 event for the digest instead of pushing it (channel sink never runs),
//! a P0 event still delivers, the default **shadow** mode changes nothing,
//! and a registry that does not name the user's persona applies no gate —
//! including the case where the snapshot's Casual fallback would have
//! borrowed the wrong floor.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

#[cfg(feature = "client-notifications")]
mod persona_gate_tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use uuid::Uuid;

    use crate::common::{create_test_server_resources, create_test_tenant};
    use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
    use pierre_core::feature_flags::FeatureKey;
    use pierre_core::models::{CoachingPersona, User};
    use pierre_database::backends::factory::Database;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::models::{Notification, NotificationCategory};
    use pierre_notifications::{
        DigestCadence, DispatchOutcome, DispatchRequest, NotificationChannelSink,
        NotificationService, PersonaPolicyGate, PushPolicy, PushTier, TenantId as CommTenantId,
        PERSONA_GATED_DATA_KEY,
    };
    use pierre_services::persona_notification_policy_gate::PersonaNotificationPolicyGate;

    /// A sink that records deliveries, so "the push side ran" is asserted by
    /// content rather than log lines.
    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl NotificationChannelSink for RecordingSink {
        async fn deliver(&self, request: &DispatchRequest) {
            self.seen
                .lock()
                .unwrap()
                .push(request.notification_type.clone());
        }
    }

    /// A gate that answers a fixed policy — the facade contract in isolation.
    struct StubGate {
        policy: PushPolicy,
    }

    #[async_trait]
    impl PersonaPolicyGate for StubGate {
        async fn policy_for(&self, _user_id: Uuid, _tenant_id: CommTenantId) -> Option<PushPolicy> {
            Some(self.policy.clone())
        }
    }

    fn casual_policy(armed: bool) -> PushPolicy {
        PushPolicy {
            persona: "casual".to_owned(),
            floor: Some(PushTier::P0),
            digest: Some(DigestCadence::Weekly),
            armed,
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

    /// The contremaitre overlay shape the platform syncs: casual promises a
    /// P0 floor with a weekly digest.
    const CASUAL_P0_WEEKLY: &str = r"
version: 1
personas:
  casual:
    notification:
      tier_floor: P0
      digest: weekly
";

    async fn seed_user(resources: &ServerContext, email: &str) -> (User, CommTenantId) {
        let (user, _token) = create_test_tenant(resources, email).await.unwrap();
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
        (user, CommTenantId(tenant.as_uuid()))
    }

    fn request(
        user_id: Uuid,
        tenant: CommTenantId,
        category: NotificationCategory,
        notification_type: &str,
    ) -> DispatchRequest {
        DispatchRequest {
            user_id,
            tenant_id: tenant,
            category,
            notification_type: notification_type.to_owned(),
            title: "Training load elevated".to_owned(),
            body: "Your acute training load is 92 — consider a recovery day".to_owned(),
            data: Some(json!({ "screen": "recovery" })),
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        }
    }

    async fn find_row(
        service: &NotificationService,
        user_id: Uuid,
        tenant: CommTenantId,
        notification_type: &str,
    ) -> Option<Notification> {
        let (rows, _, _) = service
            .list_notifications(user_id, tenant, 100, 0, None, false)
            .await
            .unwrap();
        rows.into_iter()
            .find(|n| n.notification_type == notification_type)
    }

    fn is_persona_gated(row: &Notification) -> bool {
        row.data
            .as_ref()
            .and_then(|d| d.get(PERSONA_GATED_DATA_KEY))
            .and_then(Value::as_bool)
            == Some(true)
    }

    // ════════════════════════════════════════════════════════════════
    // Facade contract, gate stubbed
    // ════════════════════════════════════════════════════════════════

    /// Armed + tier above floor ⇒ the row is persisted with the gated marker
    /// and neither push nor the channel sink runs.
    #[tokio::test]
    async fn armed_gate_persists_the_row_and_skips_the_sink() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, tenant) = seed_user(&resources, "gate_armed@example.com").await;

        let sink = Arc::new(RecordingSink::default());
        let service = notification_service(&resources.coach.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>)
            .with_policy_gate(Arc::new(StubGate {
                policy: casual_policy(true),
            }));

        let outcome = service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Training,
                    "training_load_alert",
                ),
                PushTier::P2,
            )
            .await
            .unwrap();

        assert!(
            matches!(outcome, DispatchOutcome::PersistedNoDevices { .. }),
            "gated dispatch persists without push: {outcome:?}"
        );
        assert!(
            sink.seen.lock().unwrap().is_empty(),
            "a persona-gated notification must not reach the channel sink"
        );

        let row = find_row(&service, user.id, tenant, "training_load_alert")
            .await
            .expect("the gated notification is persisted for the digest");
        assert!(
            is_persona_gated(&row),
            "the persisted row carries the persona_gated marker: {:?}",
            row.data
        );
        assert_eq!(
            row.data.as_ref().and_then(|d| d.get("screen")),
            Some(&json!("recovery")),
            "gating augments the payload, it does not replace it"
        );
    }

    /// Shadow mode (flag off) ⇒ the same event delivers exactly as before the
    /// gate existed: sink runs, no gated marker.
    #[tokio::test]
    async fn shadow_gate_delivers_and_marks_nothing() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, tenant) = seed_user(&resources, "gate_shadow@example.com").await;

        let sink = Arc::new(RecordingSink::default());
        let service = notification_service(&resources.coach.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>)
            .with_policy_gate(Arc::new(StubGate {
                policy: casual_policy(false),
            }));

        let outcome = service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Training,
                    "training_load_alert",
                ),
                PushTier::P2,
            )
            .await
            .unwrap();

        assert!(
            matches!(outcome, DispatchOutcome::PersistedNoDevices { .. }),
            "shadow mode runs the normal pipeline: {outcome:?}"
        );
        assert_eq!(
            sink.seen.lock().unwrap().as_slice(),
            ["training_load_alert"],
            "shadow mode still delivers to the channel sink"
        );
        let row = find_row(&service, user.id, tenant, "training_load_alert")
            .await
            .expect("shadow dispatch persists normally");
        assert!(
            !is_persona_gated(&row),
            "shadow mode must not mark rows as gated: {:?}",
            row.data
        );
    }

    // ════════════════════════════════════════════════════════════════
    // The real gate: persona + contremaitre contract + feature flag
    // ════════════════════════════════════════════════════════════════

    /// Sets up an armed casual user against the real
    /// [`PersonaNotificationPolicyGate`] and a hydrated contract registry.
    async fn armed_casual_service(
        resources: &Arc<ServerContext>,
        email: &str,
        overlay: &str,
        armed: bool,
    ) -> (NotificationService, Arc<RecordingSink>, User, CommTenantId) {
        let (user, tenant) = seed_user(resources, email).await;
        resources
            .common
            .repos
            .users
            .set_coaching_persona(user.id, CoachingPersona::Casual)
            .await
            .unwrap();
        if armed {
            resources
                .common
                .repos
                .feature_flags
                .set_user_override(user.id, FeatureKey::PersonaNotificationPolicy, true, None)
                .await
                .unwrap();
        }

        let registry = Arc::new(PersonaContractRegistry::new());
        if !overlay.is_empty() {
            registry.apply_overlay(overlay).unwrap();
        }

        let sink = Arc::new(RecordingSink::default());
        let service = notification_service(&resources.coach.database)
            .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>)
            .with_policy_gate(Arc::new(PersonaNotificationPolicyGate::new(
                Arc::clone(&resources.common.repos),
                registry,
            )));
        (service, sink, user, tenant)
    }

    /// The registre#7 promise end-to-end: an armed casual athlete's P0 floor
    /// holds a P2 alert AND a P1 coach message (floor P0 delivers ONLY P0),
    /// while a P0 event goes straight through.
    #[tokio::test]
    async fn armed_casual_floor_p0_delivers_only_p0() {
        let resources = create_test_server_resources().await.unwrap();
        let (service, sink, user, tenant) = armed_casual_service(
            &resources,
            "casual_armed@example.com",
            CASUAL_P0_WEEKLY,
            true,
        )
        .await;

        // P2 advisory: gated.
        let outcome = service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Training,
                    "training_load_alert",
                ),
                PushTier::P2,
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            DispatchOutcome::PersistedNoDevices { .. }
        ));
        let row = find_row(&service, user.id, tenant, "training_load_alert")
            .await
            .expect("gated P2 row persisted");
        assert!(is_persona_gated(&row), "P2 > floor P0 ⇒ gated");

        // P1 coach message: ALSO gated — floor P0 means only P0 delivers.
        let outcome = service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Coach,
                    "coach_message",
                ),
                PushTier::P1,
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            DispatchOutcome::PersistedNoDevices { .. }
        ));
        let row = find_row(&service, user.id, tenant, "coach_message")
            .await
            .expect("gated P1 row persisted");
        assert!(is_persona_gated(&row), "P1 > floor P0 ⇒ gated too");
        assert!(
            sink.seen.lock().unwrap().is_empty(),
            "neither gated event may reach the channel sink"
        );

        // P0 break-glass: delivers.
        service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::System,
                    "safety_alert",
                ),
                PushTier::P0,
            )
            .await
            .unwrap();
        assert_eq!(
            sink.seen.lock().unwrap().as_slice(),
            ["safety_alert"],
            "P0 ≤ floor P0 ⇒ delivered to the sink"
        );
        let row = find_row(&service, user.id, tenant, "safety_alert")
            .await
            .expect("delivered P0 row persisted");
        assert!(!is_persona_gated(&row), "a delivered row carries no marker");
    }

    /// Flag off ⇒ today's behavior, byte for byte: the P2 alert delivers and
    /// nothing is marked. This is the shadow-launch regression guard.
    #[tokio::test]
    async fn flag_off_keeps_todays_delivery() {
        let resources = create_test_server_resources().await.unwrap();
        let (service, sink, user, tenant) = armed_casual_service(
            &resources,
            "casual_flag_off@example.com",
            CASUAL_P0_WEEKLY,
            false,
        )
        .await;

        service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Training,
                    "training_load_alert",
                ),
                PushTier::P2,
            )
            .await
            .unwrap();
        assert_eq!(
            sink.seen.lock().unwrap().as_slice(),
            ["training_load_alert"],
            "default-off flag must not change delivery"
        );
        let row = find_row(&service, user.id, tenant, "training_load_alert")
            .await
            .unwrap();
        assert!(!is_persona_gated(&row));
    }

    /// A registry that never synced applies no gate, armed or not.
    #[tokio::test]
    async fn empty_registry_applies_no_gate() {
        let resources = create_test_server_resources().await.unwrap();
        let (service, sink, user, tenant) =
            armed_casual_service(&resources, "casual_empty_reg@example.com", "", true).await;

        service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Training,
                    "training_load_alert",
                ),
                PushTier::P2,
            )
            .await
            .unwrap();
        assert_eq!(
            sink.seen.lock().unwrap().as_slice(),
            ["training_load_alert"],
            "an unhydrated registry must not suppress anything"
        );
    }

    /// A persona the snapshot does not name gets NO policy — the snapshot's
    /// Casual fallback must not lend this user Casual's near-mute P0 floor.
    #[tokio::test]
    async fn persona_missing_from_snapshot_never_borrows_casuals_floor() {
        let resources = create_test_server_resources().await.unwrap();
        let (service, sink, user, tenant) = armed_casual_service(
            &resources,
            "power_no_contract@example.com",
            CASUAL_P0_WEEKLY,
            true,
        )
        .await;
        // Flip the persona to one the overlay does not define.
        resources
            .common
            .repos
            .users
            .set_coaching_persona(user.id, CoachingPersona::PowerAthlete)
            .await
            .unwrap();

        service
            .dispatch_with_tier(
                &request(
                    user.id,
                    tenant,
                    NotificationCategory::Training,
                    "training_load_alert",
                ),
                PushTier::P3,
            )
            .await
            .unwrap();
        assert_eq!(
            sink.seen.lock().unwrap().as_slice(),
            ["training_load_alert"],
            "a persona absent from the snapshot is permissive, never Casual's floor"
        );
        let row = find_row(&service, user.id, tenant, "training_load_alert")
            .await
            .unwrap();
        assert!(!is_persona_gated(&row));
    }
}
