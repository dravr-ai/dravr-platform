// ABOUTME: Links made through the deployment bot show up, and unlink, in the athlete's own Settings
// ABOUTME: Covers the platform-scope config rule, OTP + deep-link ingress, and per-user link isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::too_many_lines)]

//! The deployment's bot is seeded from the environment under the admin's
//! tenant, ingress writes every link it makes under that bot tenant, and a
//! self-registered athlete lives in a personal tenant of their own. These
//! tests keep those three tenants apart on purpose — the shape the older
//! single-tenant fixtures could not see — and drive the athlete's Settings
//! routes over the real router.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod bot_links_settings {
    use crate::common::{create_test_server_resources, create_test_tenant, generate_test_token};
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::{Duration, Utc};
    use hmac::{Hmac, Mac};
    use pierre_core::models::{TenantId, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_email::ResendEmailService;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::messaging_seed::seed_from_env;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_routes_auth::AuthRoutes;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::env;
    use std::sync::Arc;
    use uuid::Uuid;

    const ADMIN_EMAIL: &str = "bot-admin-439@example.com";
    const TELEGRAM_SECRET: &str = "tg_platform_secret_439";
    const WHATSAPP_SECRET: &str = "wa_platform_secret_439";
    const WHATSAPP_PHONE_ID: &str = "15550439000";

    /// Resources with an email service present, so an unlinked sender is
    /// routed into the in-chat OTP flow rather than handed a link URL. The key
    /// is fake: the tests put the OTP state in place directly and never send.
    async fn resources_with_email() -> Arc<ServerContext> {
        let mut resources = create_test_server_resources().await.unwrap();
        Arc::get_mut(&mut resources)
            .expect("no other handle to the fresh ServerContext yet")
            .common
            .email_service = Some(Arc::new(
            ResendEmailService::new(
                "re_fake_test_api_key".to_owned(),
                "Pierre <noreply@test.pierre.dev>".to_owned(),
            )
            .expect("a non-empty key builds the email service"),
        ));
        resources
    }

    async fn first_tenant(resources: &ServerContext, user_id: Uuid) -> TenantId {
        resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap()
            .first()
            .expect("user belongs to a tenant")
            .id
    }

    /// Register an athlete through the real `/api/auth/register` handler —
    /// which gives them a personal tenant — and return their id, bearer token
    /// and that tenant.
    async fn register_athlete(
        resources: &Arc<ServerContext>,
        email: &str,
    ) -> (Uuid, String, TenantId) {
        let resp = AxumTestRequest::post("/api/auth/register")
            .json(&json!({
                "email": email,
                "password": "athletePassword439",
                "display_name": "Bot Linked Athlete"
            }))
            .send(AuthRoutes::routes(resources.auth_routes_context()))
            .await;
        assert_eq!(resp.status(), 201, "registration must succeed");
        let body: Value = resp.json();
        let user_id = Uuid::parse_str(body["user_id"].as_str().unwrap()).unwrap();

        let users = &resources.common.repos.users;
        let mut user = users.get_by_email(email).await.unwrap().unwrap();
        if user.user_status != UserStatus::Active {
            user = users
                .update_status(user_id, UserStatus::Active, Some(user_id))
                .await
                .unwrap();
        }
        let tenant = first_tenant(resources, user_id).await;
        let token = generate_test_token(resources, &user).await;
        (user_id, format!("Bearer {token}"), tenant)
    }

    fn hash_otp(code: &str) -> String {
        hex::encode(Sha256::digest(code.as_bytes()))
    }

    fn whatsapp_signature(body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(WHATSAPP_SECRET.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    async fn send_telegram_text(resources: &Arc<ServerContext>, sender: i64, text: &str) {
        let body = json!({
            "update_id": 43900,
            "message": {
                "message_id": 439,
                "from": { "id": sender, "first_name": "Athlete" },
                "chat": { "id": sender, "type": "private" },
                "text": text
            }
        });
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", TELEGRAM_SECRET)
            .json(&body)
            .send(MessagingRoutes::routes(Arc::clone(resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    async fn send_whatsapp_text(resources: &Arc<ServerContext>, sender: &str, text: &str) {
        let body = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "wa_platform_business_439",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+15550439000",
                            "phone_number_id": WHATSAPP_PHONE_ID
                        },
                        "messages": [{
                            "from": sender,
                            "id": "wamid.platform_439",
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": { "body": text }
                        }]
                    },
                    "field": "messages"
                }]
            }]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let resp = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &whatsapp_signature(&bytes))
            .json(&body)
            .send(MessagingRoutes::routes(Arc::clone(resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    async fn get_json(resources: &Arc<ServerContext>, path: &str, token: &str) -> Value {
        let resp = AxumTestRequest::get(path)
            .header("authorization", token)
            .send(MessagingRoutes::routes(Arc::clone(resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "GET {path}");
        resp.json()
    }

    fn links_of(body: &Value) -> Vec<Value> {
        body["links"].as_array().expect("links array").clone()
    }

    fn available_channels(body: &Value) -> Vec<String> {
        body.as_array()
            .expect("available channels array")
            .iter()
            .map(|c| c["channel"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The whole path an athlete takes: the deployment bot is seeded from the
    /// environment under the admin's tenant, the athlete registers into a
    /// personal tenant, links Telegram through the bot's in-chat OTP, and then
    /// Settings shows it, offers the bot's channels, issues a `WhatsApp` code the
    /// bot redeems, and unlinks.
    #[tokio::test]
    async fn bot_made_links_are_listed_offered_linkable_and_unlinkable() {
        let resources = resources_with_email().await;
        let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

        // The deployment's admin, whose tenant stores the env-seeded bot.
        let (admin, _admin_token) = create_test_tenant(&resources, ADMIN_EMAIL).await.unwrap();
        let bot_tenant = first_tenant(&resources, admin.id).await;

        // The only test in this binary that touches these variables, and it
        // removes them again before asserting anything.
        let vars = [
            ("ADMIN_EMAIL", ADMIN_EMAIL),
            ("TELEGRAM_BOT_TOKEN", "439:platform-bot-token"),
            ("TELEGRAM_WEBHOOK_SECRET", TELEGRAM_SECRET),
            ("META_WHATSAPP_APP_SECRET", WHATSAPP_SECRET),
            ("META_WHATSAPP_ACCESS_TOKEN", "wa_platform_access_439"),
            ("META_WHATSAPP_PHONE_NUMBER_ID", WHATSAPP_PHONE_ID),
        ];
        for (key, value) in vars {
            env::set_var(key, value);
        }
        seed_from_env(&resources.common.repos).await;
        for (key, _) in vars {
            env::remove_var(key);
        }

        let (athlete_id, athlete_token, athlete_tenant) =
            register_athlete(&resources, "bot-linked-athlete-439@example.com").await;
        assert_ne!(
            athlete_tenant, bot_tenant,
            "the athlete must live in a personal tenant, apart from the bot's"
        );

        // Link Telegram through the bot's in-chat OTP. Ingress keeps the OTP
        // state under the bot tenant; the state is placed at the "awaiting
        // code" step so no mail is sent, and the correct code goes in through
        // the real webhook.
        let tg_sender: i64 = 4_390_001;
        let tg_sender_str = tg_sender.to_string();
        let state_id = Uuid::new_v4().to_string();
        let state_code = Uuid::new_v4().to_string();
        db.create_link_state(&CreateLinkStateParams {
            id: &state_id,
            tenant_id: bot_tenant,
            user_id: None,
            channel_type: "telegram",
            code: &state_code,
            method: "otp",
            channel_user_id: Some(&tg_sender_str),
            sender_name: Some("Athlete"),
            expires_at: &(Utc::now() + Duration::minutes(10)).to_rfc3339(),
        })
        .await
        .unwrap();
        db.set_otp_on_link_state(
            &state_id,
            "bot-linked-athlete-439@example.com",
            &hash_otp("439439"),
        )
        .await
        .unwrap();
        send_telegram_text(&resources, tg_sender, "439439").await;

        let tg_link = db
            .get_channel_link(bot_tenant, "telegram", &tg_sender_str)
            .await
            .unwrap()
            .expect("ingress writes the OTP link under the bot tenant");
        assert_eq!(tg_link["user_id"], athlete_id.to_string());

        // Settings: the bot-made link is the athlete's, whatever tenant holds it.
        let links = links_of(&get_json(&resources, "/api/messaging/links", &athlete_token).await);
        assert_eq!(links.len(), 1, "exactly the Telegram link: {links:?}");
        assert_eq!(links[0]["channel"], "telegram");
        assert_eq!(links[0]["channel_user_id"], tg_sender_str.as_str());
        assert_eq!(links[0]["display_name"], "Bot Linked Athlete");

        // Settings: the deployment's bots are offered to a personal tenant.
        let available = available_channels(
            &get_json(
                &resources,
                "/api/messaging/channels/available",
                &athlete_token,
            )
            .await,
        );
        assert_eq!(
            available,
            vec!["telegram".to_owned(), "whatsapp".to_owned()]
        );

        // Connect WhatsApp from Settings: the code is stored under the bot's
        // tenant, so the bot redeems it when the athlete sends it.
        let resp = AxumTestRequest::post("/api/messaging/link/init/whatsapp")
            .header("authorization", &athlete_token)
            .send(MessagingRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let init: Value = resp.json();
        let code = init["code"].as_str().expect("deep-link code").to_owned();
        assert_eq!(
            init["linking_url"],
            format!("https://wa.me/{WHATSAPP_PHONE_ID}?text=LINK%20{code}")
        );
        let pending = db.get_link_state(&code).await.unwrap().unwrap();
        assert_eq!(pending["tenant_id"], bot_tenant.to_string());
        assert_eq!(pending["user_id"], athlete_id.to_string());

        let wa_sender = "15550439111";
        send_whatsapp_text(&resources, wa_sender, &format!("LINK {code}")).await;
        let wa_link = db
            .get_channel_link(bot_tenant, "whatsapp", wa_sender)
            .await
            .unwrap()
            .expect("the bot redeemed the Settings-issued code");
        assert_eq!(wa_link["user_id"], athlete_id.to_string());

        let links = links_of(&get_json(&resources, "/api/messaging/links", &athlete_token).await);
        let channels: Vec<&str> = links
            .iter()
            .map(|l| l["channel"].as_str().unwrap())
            .collect();
        assert_eq!(channels, vec!["telegram", "whatsapp"]);

        // Unlink Telegram from Settings: the bot-tenant row goes, WhatsApp stays.
        let resp = AxumTestRequest::delete("/api/messaging/links/telegram")
            .header("authorization", &athlete_token)
            .send(MessagingRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: Value = resp.json();
        assert_eq!(body["status"], "unlinked");
        assert!(db
            .get_channel_link(bot_tenant, "telegram", &tg_sender_str)
            .await
            .unwrap()
            .is_none());

        let links = links_of(&get_json(&resources, "/api/messaging/links", &athlete_token).await);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["channel"], "whatsapp");
        assert_eq!(links[0]["channel_user_id"], wa_sender);
    }

    /// The web login page the bot links to when no email service is
    /// configured: a returning athlete from a personal tenant completes the
    /// link through the deployment bot, the link is stored under the bot's
    /// tenant naming the athlete (the shape the in-chat OTP flow writes), and
    /// Settings lists it. A bot another tenant keeps to itself still refuses
    /// an outsider, and a wrong password still links nothing.
    #[tokio::test]
    async fn a_personal_tenant_athlete_links_through_the_bot_login_page() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

        let (bot_admin, _) = create_test_tenant(&resources, "bot-page-admin-439@example.com")
            .await
            .unwrap();
        let bot_tenant = first_tenant(&resources, bot_admin.id).await;
        let (private_owner, _) =
            create_test_tenant(&resources, "private-bot-owner-439@example.com")
                .await
                .unwrap();
        let private_tenant = first_tenant(&resources, private_owner.id).await;

        for (tenant, channel, bot_token, phone) in [
            (bot_tenant, "telegram", Some("439:page-platform-bot"), None),
            (private_tenant, "whatsapp", None, Some("15550439333")),
        ] {
            db.upsert_channel_config(&UpsertChannelConfigParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: tenant,
                channel_type: channel,
                api_key: None,
                api_secret: None,
                webhook_secret: Some("page_secret_439"),
                verify_token: None,
                account_id: None,
                phone_number: phone,
                bot_token,
                is_active: true,
            })
            .await
            .unwrap();
        }
        assert!(db
            .mark_channel_config_platform_scope(bot_tenant, "telegram")
            .await
            .unwrap());

        let athlete_email = "bot-page-athlete-439@example.com";
        let (athlete_id, athlete_token, athlete_tenant) =
            register_athlete(&resources, athlete_email).await;
        assert_ne!(athlete_tenant, bot_tenant);

        // What the webhook stores for an unlinked sender: a code under the bot's
        // tenant naming the chat, and no user yet.
        let issue_code = |tenant: TenantId, channel: &'static str, chat: &'static str| {
            let code = Uuid::new_v4().to_string();
            async move {
                db.create_link_state(&CreateLinkStateParams {
                    id: &Uuid::new_v4().to_string(),
                    tenant_id: tenant,
                    user_id: None,
                    channel_type: channel,
                    code: &code,
                    method: "channel_initiated",
                    channel_user_id: Some(chat),
                    sender_name: Some("Page Athlete"),
                    expires_at: &(Utc::now() + Duration::minutes(10)).to_rfc3339(),
                })
                .await
                .unwrap();
                code
            }
        };
        let submit = |code: String, password: &'static str| {
            let resources = Arc::clone(&resources);
            async move {
                let resp = AxumTestRequest::post("/messaging/link/auth")
                    .form(&[
                        ("code", code.as_str()),
                        ("email", athlete_email),
                        ("password", password),
                        ("action", "login"),
                    ])
                    .send(MessagingRoutes::routes(resources))
                    .await;
                assert_eq!(resp.status_code(), StatusCode::OK);
                resp.text()
            }
        };

        // A wrong password links nothing, whichever bot issued the code.
        let code = issue_code(bot_tenant, "telegram", "tg-page-439").await;
        let page = submit(code.clone(), "notThePassword439").await;
        assert!(page.contains("Invalid email or password."), "{page}");
        assert!(db
            .get_channel_link(bot_tenant, "telegram", "tg-page-439")
            .await
            .unwrap()
            .is_none());

        // The right password completes the link through the deployment bot.
        let page = submit(code, "athletePassword439").await;
        assert!(
            !page.contains("does not belong to this organization"),
            "the deployment bot serves a personal-tenant athlete: {page}"
        );
        let link = db
            .get_channel_link(bot_tenant, "telegram", "tg-page-439")
            .await
            .unwrap()
            .expect("the link is stored under the bot's tenant");
        assert_eq!(link["user_id"], athlete_id.to_string());
        assert_eq!(link["tenant_id"], bot_tenant.to_string());

        let links = links_of(&get_json(&resources, "/api/messaging/links", &athlete_token).await);
        assert_eq!(links.len(), 1, "exactly the page-made link: {links:?}");
        assert_eq!(links[0]["channel"], "telegram");
        assert_eq!(links[0]["channel_user_id"], "tg-page-439");

        // A bot another tenant keeps to itself does not serve the athlete.
        let code = issue_code(private_tenant, "whatsapp", "15550439444").await;
        let page = submit(code, "athletePassword439").await;
        assert!(
            page.contains("does not belong to this organization"),
            "a tenant's own bot refuses an outsider: {page}"
        );
        assert!(db
            .get_channel_link(private_tenant, "whatsapp", "15550439444")
            .await
            .unwrap()
            .is_none());
        let links = links_of(&get_json(&resources, "/api/messaging/links", &athlete_token).await);
        assert_eq!(links.len(), 1, "still only the Telegram link: {links:?}");
    }

    /// One user's Settings never shows or removes another user's link, even
    /// when both were made through the same bot under the same tenant.
    #[tokio::test]
    async fn a_user_cannot_see_or_delete_another_users_link() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

        let (bot_admin, _) = create_test_tenant(&resources, "bot-owner-iso-439@example.com")
            .await
            .unwrap();
        let bot_tenant = first_tenant(&resources, bot_admin.id).await;
        let (user_a, token_a) = create_test_tenant(&resources, "athlete-a-439@example.com")
            .await
            .unwrap();
        let (user_b, token_b) = create_test_tenant(&resources, "athlete-b-439@example.com")
            .await
            .unwrap();
        let token_a = format!("Bearer {token_a}");
        let token_b = format!("Bearer {token_b}");

        for (user, channel, channel_user_id) in [
            (user_a.id, "telegram", "tg-a-439"),
            (user_b.id, "telegram", "tg-b-439"),
            (user_b.id, "whatsapp", "wa-b-439"),
        ] {
            db.create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: bot_tenant,
                user_id: &user.to_string(),
                channel_type: channel,
                channel_user_id,
                display_name: None,
            })
            .await
            .unwrap();
        }

        let links_a = links_of(&get_json(&resources, "/api/messaging/links", &token_a).await);
        assert_eq!(links_a.len(), 1, "A sees only A's link: {links_a:?}");
        assert_eq!(links_a[0]["channel_user_id"], "tg-a-439");

        // A has no WhatsApp link; B's must stay out of reach.
        let resp = AxumTestRequest::delete("/api/messaging/links/whatsapp")
            .header("authorization", &token_a)
            .send(MessagingRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);
        assert!(db
            .get_channel_link(bot_tenant, "whatsapp", "wa-b-439")
            .await
            .unwrap()
            .is_some());

        // A unlinking Telegram removes A's row and leaves B's.
        let resp = AxumTestRequest::delete("/api/messaging/links/telegram")
            .header("authorization", &token_a)
            .send(MessagingRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        assert!(db
            .get_channel_link(bot_tenant, "telegram", "tg-a-439")
            .await
            .unwrap()
            .is_none());

        let links_b = links_of(&get_json(&resources, "/api/messaging/links", &token_b).await);
        let mut ids: Vec<&str> = links_b
            .iter()
            .map(|l| l["channel_user_id"].as_str().unwrap())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["tg-b-439", "wa-b-439"]);
    }

    /// The config that serves a tenant is its own when it has one, else the
    /// platform-scope config; a config another tenant keeps to itself is never
    /// offered.
    #[tokio::test]
    async fn resolution_prefers_the_tenants_own_config_and_hides_unmarked_ones() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

        let (owner, _) = create_test_tenant(&resources, "platform-owner-439@example.com")
            .await
            .unwrap();
        let platform_tenant = first_tenant(&resources, owner.id).await;
        let (athlete, athlete_token) =
            create_test_tenant(&resources, "own-bot-athlete-439@example.com")
                .await
                .unwrap();
        let athlete_tenant = first_tenant(&resources, athlete.id).await;
        let athlete_token = format!("Bearer {athlete_token}");

        for (channel, bot_token, phone) in [
            ("telegram", Some("439:platform-telegram"), None),
            ("whatsapp", None, Some("15550439222")),
        ] {
            db.upsert_channel_config(&UpsertChannelConfigParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: platform_tenant,
                channel_type: channel,
                api_key: None,
                api_secret: None,
                webhook_secret: Some("platform_secret_439"),
                verify_token: None,
                account_id: None,
                phone_number: phone,
                bot_token,
                is_active: true,
            })
            .await
            .unwrap();
        }
        // Only Telegram is the deployment's; WhatsApp stays the owner tenant's own.
        assert!(db
            .mark_channel_config_platform_scope(platform_tenant, "telegram")
            .await
            .unwrap());
        assert!(!db
            .mark_channel_config_platform_scope(platform_tenant, "discord")
            .await
            .unwrap());

        let resolved = db
            .resolve_channel_config(athlete_tenant, "telegram")
            .await
            .unwrap()
            .expect("platform Telegram serves every tenant");
        assert_eq!(resolved["tenant_id"], platform_tenant.to_string());
        assert_eq!(resolved["bot_token"], "439:platform-telegram");
        assert!(db
            .resolve_channel_config(athlete_tenant, "whatsapp")
            .await
            .unwrap()
            .is_none());

        let available = available_channels(
            &get_json(
                &resources,
                "/api/messaging/channels/available",
                &athlete_token,
            )
            .await,
        );
        assert_eq!(available, vec!["telegram".to_owned()]);

        // The athlete's tenant brings its own Telegram bot and switches it off:
        // its own config decides, and the platform bot is not offered instead.
        let resp = AxumTestRequest::put("/api/messaging/channels/telegram")
            .header("authorization", &athlete_token)
            .json(&json!({
                "enabled": false,
                "credentials": { "bot_token": "439:tenant-own-telegram" }
            }))
            .send(MessagingRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let resolved = db
            .resolve_channel_config(athlete_tenant, "telegram")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved["tenant_id"], athlete_tenant.to_string());
        assert_eq!(resolved["bot_token"], "439:tenant-own-telegram");
        let available = available_channels(
            &get_json(
                &resources,
                "/api/messaging/channels/available",
                &athlete_token,
            )
            .await,
        );
        assert!(available.is_empty(), "got {available:?}");

        // Init refuses a disabled bot rather than issue a code nobody redeems.
        let resp = AxumTestRequest::post("/api/messaging/link/init/telegram")
            .header("authorization", &athlete_token)
            .send(MessagingRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
        let body: Value = resp.json();
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| m.contains("No telegram bot is available")),
            "the refusal names the channel: {body}"
        );
    }
}
