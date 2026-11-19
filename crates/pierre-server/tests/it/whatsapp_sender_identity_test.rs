// ABOUTME: A WhatsApp sender is routed on their business-scoped user id; placeholder ids are never linked or routed
// ABOUTME: An athlete linked by phone number keeps their account and session when Meta starts naming them by BSUID
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1668. Meta's username rollout made the phone number in
//! `messages[].from` optional. canot up to 0.11 read `from` alone and wrote
//! the literal `"unknown"` when it was missing, so every athlete with a
//! username who was new to the business number became one sender: the first
//! to link bound `"unknown"` to their account, and every later one was routed
//! into it, reading its coaching history and writing into its conversation.
//!
//! canot keys a `WhatsApp` sender on the business-scoped user id (BSUID).
//! These suites pin the platform half: a phone-number link moves onto the
//! BSUID with its session, two username senders stay two senders, and a
//! placeholder id is neither linked nor routed, even where a link already
//! holds it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(feature = "client-messaging")]
mod whatsapp_sender_identity_tests {
    use crate::common::{create_test_server_resources, create_test_user_with_plan};
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::messaging_fixtures::seed_channel_link;
    use axum::http::StatusCode;
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::TenantId;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateSessionParams, MessagingRepository,
        RekeyChannelIdentityParams, UpsertChannelConfigParams,
    };
    use pierre_database::repositories::{
        ResumableTurnRepository, ResumableTurnRow, TurnClaim, TurnLease,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::{json, Value};
    use sha2::Sha256;
    use std::sync::Arc;
    use uuid::Uuid;

    const APP_SECRET: &str = "whatsapp_identity_app_secret";
    const PHONE: &str = "15149166448";
    const BSUID_A: &str = "US.13491208655302741918";
    const BSUID_B: &str = "CA.80211354923874651002";

    /// A user owning their own tenant, which also holds the `WhatsApp`
    /// channel config: the bot tenant a webhook arrives under. Linking this
    /// user is the self-hosted shape, bot tenant = user tenant.
    async fn user_with_whatsapp(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
        let (user_id, _, tenant_id) =
            create_test_user_with_plan(&resources.agent.database, email, "starter")
                .await
                .unwrap();
        resources
            .common
            .repos
            .messaging
            .upsert_channel_config(&UpsertChannelConfigParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id,
                channel_type: "whatsapp",
                api_key: Some("whatsapp_access_token"),
                api_secret: None,
                webhook_secret: Some(APP_SECRET),
                verify_token: None,
                account_id: Some("whatsapp_business_account_id"),
                phone_number: Some("123456789012345"),
                bot_token: None,
                is_active: true,
            })
            .await
            .unwrap();
        (user_id, tenant_id)
    }

    /// An active user owning their own tenant, with no channel config.
    async fn user_with_tenant(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
        let (user_id, _, tenant_id) =
            create_test_user_with_plan(&resources.agent.database, email, "starter")
                .await
                .unwrap();
        (user_id, tenant_id)
    }

    /// Link `user` to the `WhatsApp` channel identity `identity` under `tenant_id`.
    async fn link(resources: &ServerContext, tenant_id: TenantId, user: Uuid, identity: &str) {
        seed_channel_link(
            &resources.agent.database,
            &user.to_string(),
            tenant_id,
            "whatsapp",
            identity,
        )
        .await;
    }

    async fn linked_user(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        identity: &str,
    ) -> Option<String> {
        db.get_channel_link(tenant_id, "whatsapp", identity)
            .await
            .unwrap()
            .map(|l| l["user_id"].as_str().unwrap().to_owned())
    }

    /// One signed text message, naming its sender by the given `from` /
    /// `from_user_id` fields. Returns `messages_stored` from the response.
    async fn post_text(
        resources: &Arc<ServerContext>,
        wamid: &str,
        sender: &[(&str, &str)],
    ) -> u64 {
        let mut message = json!({
            "id": wamid,
            "timestamp": "1763560000",
            "type": "text",
            "text": { "body": "How did my long run go?" }
        });
        let mut contact = json!({ "profile": { "name": "Athlete" } });
        for (field, value) in sender {
            message[*field] = json!(value);
            let key = if *field == "from" { "wa_id" } else { "user_id" };
            contact[key] = json!(value);
        }
        let body = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "whatsapp_business_account_id",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+14388392708",
                            "phone_number_id": "123456789012345"
                        },
                        "contacts": [contact],
                        "messages": [message]
                    }
                }]
            }]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(APP_SECRET.as_bytes()).unwrap();
        mac.update(&bytes);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let resp = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &signature)
            .json(&body)
            .send(MessagingRoutes::routes(Arc::clone(resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let reply: Value = resp.json();
        reply["messages_stored"].as_u64().unwrap()
    }

    /// Link states (a link prompt, an OTP flow) opened for a channel identity,
    /// read past the repository, which exposes only the live OTP flow.
    async fn link_states_for(resources: &ServerContext, identity: &str) -> i64 {
        const SQL: &str = "SELECT COUNT(*) FROM messaging_link_states \
                           WHERE channel_type = 'whatsapp' AND channel_user_id = $1";
        match resources.agent.database.backend() {
            DatabaseBackend::SQLite(backend) => sqlx::query_scalar(SQL)
                .bind(identity)
                .fetch_one(backend.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(SQL)
                .bind(identity)
                .fetch_one(backend.pool())
                .await
                .unwrap(),
        }
    }

    /// A link holding the placeholder `"unknown"`, written past the repository
    /// (which refuses it) — the row a database written by a canot parser that
    /// substituted `"unknown"` may still carry.
    async fn plant_placeholder_link(resources: &ServerContext, tenant_id: TenantId, user: Uuid) {
        match resources.agent.database.backend() {
            DatabaseBackend::SQLite(backend) => {
                sqlx::query(
                    "INSERT INTO messaging_channel_links \
                     (id, tenant_id, user_id, channel_type, channel_user_id, linked_at) \
                     VALUES ($1, $2, $3, 'whatsapp', 'unknown', $4)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(tenant_id.to_string())
                .bind(user.to_string())
                .bind(Utc::now().to_rfc3339())
                .execute(backend.pool())
                .await
                .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(backend) => {
                sqlx::query(
                    "INSERT INTO messaging_channel_links \
                     (id, tenant_id, user_id, channel_type, channel_user_id, linked_at) \
                     VALUES ($1, $2, $3, 'whatsapp', 'unknown', $4)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(tenant_id.as_uuid())
                .bind(user)
                .bind(Utc::now())
                .execute(backend.pool())
                .await
                .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn an_athlete_linked_by_phone_number_moves_onto_the_bsuid_with_their_session() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (user, tenant_id) = user_with_whatsapp(&resources, "phone@example.com").await;
        link(&resources, tenant_id, user, PHONE).await;
        let session_id = Uuid::new_v4().to_string();
        db.create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user.to_string(),
            tenant_id,
            channel_type: "whatsapp",
            channel_user_id: PHONE,
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();

        let stored = post_text(
            &resources,
            "wamid.both",
            &[("from", PHONE), ("from_user_id", BSUID_A)],
        )
        .await;

        assert_eq!(stored, 1, "the athlete is routed as linked, not prompted");
        assert_eq!(
            linked_user(db, tenant_id, BSUID_A).await,
            Some(user.to_string())
        );
        assert_eq!(linked_user(db, tenant_id, PHONE).await, None);
        let session = db
            .get_session_by_channel_identity(tenant_id, "whatsapp", BSUID_A, None)
            .await
            .unwrap()
            .expect("the session follows the link");
        assert_eq!(session["id"].as_str(), Some(session_id.as_str()));
    }

    /// The production shape: the `WhatsApp` config and the link live under the
    /// bot owner's tenant, the athlete's DM session under the athlete's own.
    #[tokio::test]
    async fn an_athlete_of_an_admin_owned_bot_moves_onto_the_bsuid_with_their_own_tenant_session() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (_, bot_tenant) = user_with_whatsapp(&resources, "bot-owner@example.com").await;
        let (athlete, athlete_tenant) = user_with_tenant(&resources, "athlete@example.com").await;
        link(&resources, bot_tenant, athlete, PHONE).await;
        let session_id = Uuid::new_v4().to_string();
        db.create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &athlete.to_string(),
            tenant_id: athlete_tenant,
            channel_type: "whatsapp",
            channel_user_id: PHONE,
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();

        let stored = post_text(
            &resources,
            "wamid.admin-bot",
            &[("from", PHONE), ("from_user_id", BSUID_A)],
        )
        .await;

        assert_eq!(stored, 1, "the athlete is routed as linked, not prompted");
        assert_eq!(
            linked_user(db, bot_tenant, BSUID_A).await,
            Some(athlete.to_string()),
            "the link moves onto the BSUID under the bot tenant"
        );
        assert_eq!(linked_user(db, bot_tenant, PHONE).await, None);
        let session = db
            .get_session_by_channel_identity(athlete_tenant, "whatsapp", BSUID_A, None)
            .await
            .unwrap()
            .expect("the DM session under the athlete's own tenant follows the link");
        assert_eq!(session["id"].as_str(), Some(session_id.as_str()));
        assert!(
            db.get_session_by_channel_identity(athlete_tenant, "whatsapp", PHONE, None)
                .await
                .unwrap()
                .is_none(),
            "no session is left keyed on the phone number"
        );
    }

    #[tokio::test]
    async fn two_username_senders_with_no_phone_number_never_share_an_account() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (user, tenant_id) = user_with_whatsapp(&resources, "first@example.com").await;
        link(&resources, tenant_id, user, BSUID_A).await;

        let first = post_text(&resources, "wamid.a", &[("from_user_id", BSUID_A)]).await;
        let second = post_text(&resources, "wamid.b", &[("from_user_id", BSUID_B)]).await;

        assert_eq!(first, 1, "the linked username sender is routed");
        assert_eq!(
            second, 0,
            "the second username sender is not routed into the first account"
        );
        assert_eq!(linked_user(db, tenant_id, BSUID_B).await, None);
        assert!(
            link_states_for(&resources, BSUID_B).await > 0,
            "the second sender is offered linking under their own BSUID"
        );
        assert_eq!(
            linked_user(db, tenant_id, BSUID_A).await,
            Some(user.to_string())
        );
    }

    #[tokio::test]
    async fn a_placeholder_sender_is_never_routed_even_where_a_link_already_holds_it() {
        let resources = create_test_server_resources().await.unwrap();
        let (user, tenant_id) = user_with_whatsapp(&resources, "owner@example.com").await;
        plant_placeholder_link(&resources, tenant_id, user).await;

        let stored = post_text(
            &resources,
            "wamid.placeholder",
            &[("from_user_id", "unknown")],
        )
        .await;

        assert_eq!(
            stored, 0,
            "a placeholder sender is not routed into the account holding it"
        );
        assert_eq!(
            link_states_for(&resources, "unknown").await,
            0,
            "a placeholder sender is not offered linking either"
        );
    }

    #[tokio::test]
    async fn the_link_write_refuses_a_placeholder_identity() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (user, tenant_id) = user_with_tenant(&resources, "refused@example.com").await;

        for identity in ["unknown", "UNKNOWN", "", "  "] {
            let err = db
                .create_channel_link(&CreateChannelLinkParams {
                    id: &Uuid::new_v4().to_string(),
                    tenant_id,
                    user_id: &user.to_string(),
                    channel_type: "whatsapp",
                    channel_user_id: identity,
                    display_name: None,
                })
                .await
                .expect_err("a placeholder identity is refused");
            assert_eq!(err.code, ErrorCode::InvalidInput, "{identity:?}");
        }
        assert_eq!(linked_user(db, tenant_id, "unknown").await, None);
    }

    #[tokio::test]
    async fn a_rekey_never_takes_an_identity_another_link_holds() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (phone_user, tenant_id) =
            user_with_tenant(&resources, "phone-holder@example.com").await;
        let (bsuid_user, _) = user_with_tenant(&resources, "bsuid-holder@example.com").await;
        link(&resources, tenant_id, phone_user, PHONE).await;
        link(&resources, tenant_id, bsuid_user, BSUID_A).await;

        let moved = db
            .rekey_channel_identity(&RekeyChannelIdentityParams {
                link_tenant_id: tenant_id,
                session_tenant_id: tenant_id,
                user_id: &phone_user.to_string(),
                channel_type: "whatsapp",
                from_channel_user_id: PHONE,
                to_channel_user_id: BSUID_A,
            })
            .await
            .unwrap();

        assert!(
            !moved,
            "a rekey onto an identity another link holds does not move"
        );
        assert_eq!(
            linked_user(db, tenant_id, PHONE).await,
            Some(phone_user.to_string())
        );
        assert_eq!(
            linked_user(db, tenant_id, BSUID_A).await,
            Some(bsuid_user.to_string())
        );
    }

    #[tokio::test]
    async fn a_rekey_moves_only_the_named_users_link() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (holder, tenant_id) = user_with_tenant(&resources, "holder@example.com").await;
        let (other, _) = user_with_tenant(&resources, "other@example.com").await;
        link(&resources, tenant_id, holder, PHONE).await;

        let moved = db
            .rekey_channel_identity(&RekeyChannelIdentityParams {
                link_tenant_id: tenant_id,
                session_tenant_id: tenant_id,
                user_id: &other.to_string(),
                channel_type: "whatsapp",
                from_channel_user_id: PHONE,
                to_channel_user_id: BSUID_A,
            })
            .await
            .unwrap();

        assert!(!moved, "a link naming another user does not move");
        assert_eq!(
            linked_user(db, tenant_id, PHONE).await,
            Some(holder.to_string())
        );
        assert_eq!(linked_user(db, tenant_id, BSUID_A).await, None);
    }

    /// A turn recorded under the phone number before the move (a deploy cut
    /// its reply off) follows the link, so it still authenticates when the
    /// sweep resumes it.
    #[tokio::test]
    async fn a_turn_recorded_under_the_phone_number_follows_the_link_onto_the_bsuid() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let turns: &dyn ResumableTurnRepository = &*resources.common.repos.resumable_turns;
        let (_, bot_tenant) = user_with_whatsapp(&resources, "turn-bot-owner@example.com").await;
        let (athlete, athlete_tenant) =
            user_with_tenant(&resources, "turn-athlete@example.com").await;
        link(&resources, bot_tenant, athlete, PHONE).await;
        let turn = ResumableTurnRow {
            id: Uuid::new_v4().to_string(),
            tenant_id: athlete_tenant.to_string(),
            channel_tenant_id: bot_tenant.to_string(),
            user_tenant_id: athlete_tenant.to_string(),
            session_id: Uuid::new_v4().to_string(),
            conversation: Uuid::new_v4().to_string(),
            user_id: athlete.to_string(),
            channel: "whatsapp".to_owned(),
            sender_id: PHONE.to_owned(),
            conversation_id: Some(PHONE.to_owned()),
            channel_message_id: "wamid.recorded".to_owned(),
            thread_id: None,
            text_content: "how did my ride go?".to_owned(),
            is_group_chat: false,
            locale: "en".to_owned(),
            turn_id: Uuid::new_v4().to_string(),
            placeholder_message_id: None,
            attempts: 0,
            enqueue_seq: 0,
            created_at_ms: 1_000,
        };
        assert!(
            turns.record_resumable_turn(&turn).await.unwrap(),
            "the turn is recorded"
        );

        let moved = db
            .rekey_channel_identity(&RekeyChannelIdentityParams {
                link_tenant_id: bot_tenant,
                session_tenant_id: athlete_tenant,
                user_id: &athlete.to_string(),
                channel_type: "whatsapp",
                from_channel_user_id: PHONE,
                to_channel_user_id: BSUID_A,
            })
            .await
            .unwrap();
        assert!(moved, "the athlete's phone-number link moves");

        let lease = TurnLease {
            leased_by: "resume-test",
            now_ms: 2_000,
            lease_until_ms: 92_000,
            max_attempts: 2,
        };
        let claim = turns
            .claim_resumable_turn(athlete_tenant, &turn.id, &lease)
            .await
            .unwrap();
        assert!(
            matches!(&claim, TurnClaim::Claimed(resumed) if resumed.sender_id == BSUID_A),
            "the recorded turn is still claimable and its sender follows the link onto the BSUID"
        );
    }
}
