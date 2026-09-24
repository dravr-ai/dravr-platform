// ABOUTME: carnet#505 — the Strava seat reclaimer warns idle athletes, then frees their seats under the runtime policy
// ABOUTME: Observe acts on nobody; enforce warns first, waits the lead, revokes at Strava, and obeys every policy knob

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every test drives the real sweeper against a real database: the seat
//! counts and holder listing the admin routes use, the admin config service
//! `pierre-cli config set` writes through, the notification service the
//! reconnect pushes use, the auth middleware every MCP and API request goes
//! through, and the disconnect chokepoint with Strava's revocation endpoint
//! pointed at a local stub, so "revoked at Strava" is asserted on the wire.
//! Each pass is run at an explicit instant, so a lead of days is crossed by
//! passing a later `now` rather than by waiting.
//!
//! The env app's cap is four seats in every test of this binary, so four
//! holders that count as seats leave none free. Every connection is seeded a
//! year before the test's first pass, as an athlete idle for weeks connected
//! long before, so a warning always postdates the connection it is about
//! unless a test reconnects on purpose.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-notifications")]
mod strava_seat_reclaim_tests {
    use std::collections::HashMap;
    use std::env;
    use std::sync::{Arc, Mutex};
    use std::time::Duration as StdDuration;

    use async_trait::async_trait;
    use chrono::{DateTime, Duration, Utc};
    use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
    use pierre_config::admin_types::{
        ConfigDataType, ConfigScope, ResetConfigRequest, UpdateConfigRequest, UpdateConfigResponse,
    };
    use pierre_config::constants::strava_seat_reclaim as keys;
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserOAuthToken, UserStatus};
    use pierre_database::backends::factory::Database;
    use pierre_database::RepositoryRegistry;
    #[cfg(feature = "postgresql")]
    use pierre_mcp_server::config::admin::postgres_manager::PostgresAdminConfigManager;
    use pierre_mcp_server::config::admin::repository::SetOverrideParams;
    use pierre_mcp_server::config::admin::{
        AdminConfigManager, AdminConfigRepository, UpdateConfigContext,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_notifications::models::{
        Notification, NotificationCategory, UpsertNotificationPreferenceParams,
    };
    use pierre_notifications::{
        DispatchRequest, NotificationChannelSink, NotificationService, TenantId as CommTenantId,
    };
    use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
    use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
    use pierre_services::oauth_flow::OAuthService;
    use pierre_services::persona_notification_policy_gate::PersonaNotificationPolicyGate;
    use pierre_services::provider_revocation::RevocationOutcome;
    use pierre_services::strava_seat_reclaim::{
        start_strava_seat_reclaimer, ReclaimAction, ReclaimMode, ReclaimReport,
        StravaSeatReclaimer, TICK_INTERVAL, WORKER_NAME,
    };
    use pierre_services::user_removal::ProviderDisconnector;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::create_test_server_resources;
    use crate::helpers::notify_capture::{capture_logs, capture_notify, named, only};

    /// The refresh token every seeded grant carries; a revocation spends it.
    const REFRESH_TOKEN: &str = "seat-reclaim-refresh-do-not-log";

    /// A stand-in for Strava's revocation endpoint that confirms every request
    /// and hands the raw bytes back. Given an athlete, it marks them active
    /// the moment the first revocation arrives and before it answers, as an
    /// athlete opening the app while the pass is still disconnecting would.
    struct RevokeStub {
        url: String,
        requests: mpsc::UnboundedReceiver<String>,
    }

    impl RevokeStub {
        async fn start() -> Self {
            Self::serve(None).await
        }

        async fn start_bumping(resources: &Arc<ServerContext>, athlete: Holder) -> Self {
            Self::serve(Some((Arc::clone(resources), athlete))).await
        }

        async fn serve(mut on_first: Option<(Arc<ServerContext>, Holder)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let mut buf = vec![0_u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if let Some((resources, athlete)) = on_first.take() {
                        set_last_active(&resources, athlete, Utc::now()).await;
                    }
                    if tx
                        .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                        .is_err()
                    {
                        return;
                    }
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = stream.shutdown().await;
                }
            });
            Self { url, requests: rx }
        }

        fn received(&mut self) -> Vec<String> {
            let mut seen = Vec::new();
            while let Ok(raw) = self.requests.try_recv() {
                seen.push(raw);
            }
            seen
        }
    }

    /// Stands in for the linked chat channel every seeded athlete has, and
    /// records what reached it. The production sink resolves the athlete's
    /// channel links and posts to hardcoded channel hosts, so a test sending
    /// through it would assert the network; its link resolution is covered by
    /// `notification_messaging_sink_test`, and the web-only test below runs
    /// the production sink for an athlete with no link, where it reaches
    /// nothing.
    #[derive(Default)]
    struct LinkedChannel {
        sent: Mutex<Vec<(Uuid, String)>>,
    }

    #[async_trait]
    impl NotificationChannelSink for LinkedChannel {
        async fn deliver(&self, request: &DispatchRequest) -> usize {
            self.sent
                .lock()
                .unwrap()
                .push((request.user_id, request.title.clone()));
            1
        }
    }

    /// One seeded Strava seat holder.
    #[derive(Clone, Copy)]
    struct Holder {
        user_id: Uuid,
        tenant_id: TenantId,
    }

    /// A server context with the env Strava app every test in this binary
    /// configures, set before the resources exist so parallel tests agree.
    async fn resources() -> Arc<ServerContext> {
        env::set_var("STRAVA_CLIENT_ID", "seat-reclaim-client");
        env::set_var("STRAVA_CLIENT_SECRET", "seat-reclaim-secret");
        env::set_var("STRAVA_OAUTH_SEAT_CAP", "4");
        create_test_server_resources().await.unwrap()
    }

    /// The notification service the server boots — pipeline, persona gate and
    /// localizer — with the athlete's linked chat channel as its sink.
    fn linked_notifications(
        resources: &ServerContext,
    ) -> (Arc<NotificationService>, Arc<LinkedChannel>) {
        let channel = Arc::new(LinkedChannel::default());
        let service = match resources.agent.database.as_ref() {
            Database::SQLite(db) => NotificationService::from_sqlite(db.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => NotificationService::from_postgres(db.pool().clone()),
        }
        .with_channel_sink(Arc::clone(&channel) as Arc<dyn NotificationChannelSink>)
        .with_policy_gate(Arc::new(PersonaNotificationPolicyGate::new(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.fitness.persona_contract_registry),
        )))
        .with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.mcp.messaging_strings_registry),
        )));
        (Arc::new(service), channel)
    }

    /// The sweeper as the boot wiring assembles it, warning through
    /// `notifications`, with Strava's revocation endpoint at `revoke_url`.
    fn reclaimer_with(
        resources: &Arc<ServerContext>,
        revoke_url: &str,
        notifications: Option<Arc<NotificationService>>,
    ) -> StravaSeatReclaimer {
        let mut config = (*resources.common.config).clone();
        revoke_url.clone_into(&mut config.external_services.strava_api.revoke_url);
        let disconnector: Arc<dyn ProviderDisconnector> =
            Arc::new(OAuthService::new(resources.data(), Arc::new(config)));
        let admin_config: Arc<dyn AdminConfigLookup> =
            resources.agent.admin_config.clone().unwrap();
        StravaSeatReclaimer::new(
            Arc::clone(&resources.common.repos),
            admin_config,
            disconnector,
            notifications,
        )
    }

    /// The sweeper warning athletes who all have a linked chat channel.
    fn reclaimer(resources: &Arc<ServerContext>, revoke_url: &str) -> StravaSeatReclaimer {
        reclaimer_with(
            resources,
            revoke_url,
            Some(linked_notifications(resources).0),
        )
    }

    /// A user with their own tenant, last active `idle` before `t0`.
    async fn seed_user(
        repos: &RepositoryRegistry,
        label: &str,
        t0: DateTime<Utc>,
        idle: Duration,
        locale: &str,
    ) -> Holder {
        let mut user = User::new(
            format!("{label}-{}@example.com", Uuid::new_v4()),
            "hash".to_owned(),
            Some(label.to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.last_active = t0 - idle;
        locale.clone_into(&mut user.locale);
        let user_id = user.id;
        repos.users.create(&user).await.unwrap();
        let tenant_id = TenantId::generate();
        repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: format!("{label} tenant"),
                slug: format!("{label}-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: user_id,
                created_at: t0,
                updated_at: t0,
            })
            .await
            .unwrap();
        Holder { user_id, tenant_id }
    }

    /// A Strava grant as the OAuth callback writes it, on the env app
    /// (`app` `None`) or a pool app, connected at `connected_at`.
    async fn connect_strava(
        resources: &ServerContext,
        holder: Holder,
        app: Option<&str>,
        connected_at: DateTime<Utc>,
    ) {
        let repos = &resources.common.repos;
        repos
            .provider_connections
            .register_connection(
                holder.user_id,
                holder.tenant_id,
                "strava",
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: holder.user_id,
                tenant_id: holder.tenant_id.to_string(),
                provider: "strava".to_owned(),
                access_token: "seat-reclaim-access-do-not-log".to_owned(),
                refresh_token: Some(REFRESH_TOKEN.to_owned()),
                token_type: "Bearer".to_owned(),
                expires_at: Some(Utc::now() + Duration::hours(6)),
                scope: Some("read,activity:read_all".to_owned()),
                provider_user_id: None,
                oauth_app_client_id: app.map(str::to_owned),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        set_connected_at(resources, holder, connected_at).await;
    }

    /// Idle `days` whole days and an hour, so no pass lands on a day boundary.
    fn idle(days: i64) -> Duration {
        Duration::days(days) + Duration::hours(1)
    }

    /// When every seeded athlete connected: long before any of them went idle.
    fn long_ago(t0: DateTime<Utc>) -> DateTime<Utc> {
        t0 - Duration::days(365)
    }

    /// An English-speaking Strava seat holder on the env app, last active
    /// `idle_days` ago.
    async fn holder(
        resources: &ServerContext,
        label: &str,
        t0: DateTime<Utc>,
        idle_days: i64,
    ) -> Holder {
        holder_on(resources, label, t0, idle_days, None).await
    }

    /// An English-speaking Strava seat holder on `app`, last active
    /// `idle_days` ago.
    async fn holder_on(
        resources: &ServerContext,
        label: &str,
        t0: DateTime<Utc>,
        idle_days: i64,
        app: Option<&str>,
    ) -> Holder {
        let holder = seed_user(&resources.common.repos, label, t0, idle(idle_days), "en").await;
        connect_strava(resources, holder, app, long_ago(t0)).await;
        holder
    }

    /// A user created to attribute config writes to.
    async fn admin(repos: &RepositoryRegistry) -> String {
        let user = User::new(
            format!("seat-reclaim-admin-{}@example.com", Uuid::new_v4()),
            "hash".to_owned(),
            None,
        );
        repos.users.create(&user).await.unwrap();
        user.id.to_string()
    }

    fn context(admin_id: &str) -> UpdateConfigContext<'_> {
        UpdateConfigContext {
            admin_user_id: admin_id,
            admin_email: "seat-reclaim-admin@example.com",
            scope: ConfigScope::Global,
            ip_address: None,
            user_agent: None,
        }
    }

    /// Write `parameters` system-wide through the service `PUT /api/admin/config`
    /// (and so `pierre-cli config set`) calls.
    async fn set_policy(
        resources: &ServerContext,
        admin_id: &str,
        parameters: &[(&str, Value)],
    ) -> UpdateConfigResponse {
        let parameters: HashMap<String, Value> = parameters
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect();
        resources
            .agent
            .admin_config
            .as_ref()
            .unwrap()
            .update_config(
                &UpdateConfigRequest {
                    parameters,
                    reason: Some("carnet#505 test".to_owned()),
                },
                context(admin_id),
            )
            .await
            .unwrap()
    }

    async fn set_policy_ok(
        resources: &ServerContext,
        admin_id: &str,
        parameters: &[(&str, Value)],
    ) {
        let response = set_policy(resources, admin_id, parameters).await;
        assert!(response.success, "{:?}", response.validation_errors);
    }

    async fn observe(resources: &ServerContext) {
        let admin_id = admin(&resources.common.repos).await;
        set_policy_ok(
            resources,
            &admin_id,
            &[(keys::MODE_KEY, json!(keys::MODE_OBSERVE))],
        )
        .await;
    }

    async fn enforce(resources: &ServerContext) {
        let admin_id = admin(&resources.common.repos).await;
        set_policy_ok(
            resources,
            &admin_id,
            &[(keys::MODE_KEY, json!(keys::MODE_ENFORCE))],
        )
        .await;
    }

    /// A stored row written straight through the repository, as a hand edit
    /// of the database would be, bypassing the catalog's validation.
    async fn hand_edit(
        resources: &ServerContext,
        key: &str,
        value: Value,
        data_type: ConfigDataType,
    ) {
        let admin_id = admin(&resources.common.repos).await;
        let repository: Box<dyn AdminConfigRepository> = match resources.agent.database.as_ref() {
            Database::SQLite(db) => Box::new(AdminConfigManager::new(db.pool().clone())),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                Box::new(PostgresAdminConfigManager::new(db.pool().clone()))
            }
        };
        repository
            .set_override(SetOverrideParams {
                category: keys::CATEGORY,
                key,
                value: &value,
                data_type,
                admin_user_id: &admin_id,
                scope: ConfigScope::Global,
                reason: Some("hand edit"),
            })
            .await
            .unwrap();
    }

    async fn notifications_of(resources: &ServerContext, holder: Holder) -> Vec<Notification> {
        let (rows, _, _) = resources
            .common
            .notification_service
            .as_ref()
            .unwrap()
            .list_notifications(
                holder.user_id,
                CommTenantId(holder.tenant_id.as_uuid()),
                50,
                0,
                None,
                false,
            )
            .await
            .unwrap();
        rows
    }

    /// When the stored warning about `holder` was sent, and whether it reached
    /// them outside the app.
    async fn warning_of(resources: &ServerContext, holder: Holder) -> Option<(i64, bool)> {
        resources
            .common
            .repos
            .strava_seat_reclaim_warnings
            .get_seat_reclaim_warning(holder.user_id, holder.tenant_id)
            .await
            .unwrap()
            .map(|w| (w.warned_at.timestamp_millis(), w.reached))
    }

    async fn holds_strava_token(resources: &ServerContext, holder: Holder) -> bool {
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(holder.user_id, holder.tenant_id, "strava")
            .await
            .unwrap()
            .is_some()
    }

    /// The planned action of `holder` in `report`, or `None` when they were
    /// not a candidate.
    fn action_of(report: &ReclaimReport, holder: Holder) -> Option<ReclaimAction> {
        report
            .candidates
            .iter()
            .find(|c| c.user_id == holder.user_id && c.tenant_id == holder.tenant_id)
            .map(|c| c.action)
    }

    fn reclaimed_users(report: &ReclaimReport) -> Vec<Uuid> {
        report.reclaimed.iter().map(|seat| seat.user_id).collect()
    }

    fn warned_users(report: &ReclaimReport) -> Vec<Uuid> {
        report.warned.iter().map(|(user_id, _)| *user_id).collect()
    }

    /// Move an athlete's `users.last_active`, as a login or a message would.
    async fn set_last_active(resources: &ServerContext, holder: Holder, at: DateTime<Utc>) {
        const SQL: &str = "UPDATE users SET last_active = $1 WHERE id = $2";
        match resources.agent.database.as_ref() {
            Database::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(at)
                    .bind(holder.user_id.to_string())
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(at)
                    .bind(holder.user_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// Set when the athlete's Strava connection was made. Both engines store
    /// this table's ids as text.
    async fn set_connected_at(resources: &ServerContext, holder: Holder, at: DateTime<Utc>) {
        const SQL: &str = "UPDATE provider_connections SET connected_at = $1 \
                           WHERE user_id = $2 AND tenant_id = $3 AND provider = 'strava'";
        let user_id = holder.user_id.to_string();
        let tenant_id = holder.tenant_id.to_string();
        match resources.agent.database.as_ref() {
            Database::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(at)
                    .bind(user_id)
                    .bind(tenant_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(at)
                    .bind(user_id)
                    .bind(tenant_id)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// Observe, the shipped default, plans exactly what enforce would do and
    /// does none of it: no notification, no warning row, no revocation, every
    /// token still in place. Its report is one log line per candidate naming
    /// the athlete by id and the action enforce would take, never an email.
    #[tokio::test]
    async fn observe_reports_the_candidates_and_acts_on_nobody() {
        let resources = resources().await;
        observe(&resources).await;
        let mut stub = RevokeStub::start().await;
        let t0 = Utc::now();
        let long_gone = holder(&resources, "long-gone", t0, 30).await;
        let gone = holder(&resources, "gone", t0, 20).await;
        let fading = holder(&resources, "fading", t0, 8).await;
        let active = holder(&resources, "active", t0, 2).await;

        let (lines, guard) = capture_logs();
        let report = reclaimer(&resources, &stub.url).tick(t0).await.unwrap();
        drop(guard);

        assert_eq!(report.policy.mode, ReclaimMode::Observe);
        assert_eq!(report.policy.idle_days, keys::DEFAULT_IDLE_DAYS);
        assert_eq!(report.free_seats, 0, "four holders fill the four seats");
        assert_eq!(report.deficit, 2, "two short of min_free_seats");
        let ordered: Vec<(Uuid, i64)> = report
            .candidates
            .iter()
            .map(|c| (c.user_id, c.idle_days))
            .collect();
        assert_eq!(
            ordered,
            vec![
                (long_gone.user_id, 30),
                (gone.user_id, 20),
                (fading.user_id, 8)
            ],
            "idle past idle_days - warn_lead_days, least recently active first"
        );
        assert_eq!(action_of(&report, long_gone), Some(ReclaimAction::Warn));
        assert_eq!(action_of(&report, gone), Some(ReclaimAction::Hold));
        assert_eq!(action_of(&report, fading), Some(ReclaimAction::Hold));
        assert_eq!(action_of(&report, active), None);

        let reported = named(&lines, "strava seat reclaim candidate");
        let logged: Vec<(String, String, String, String)> = reported
            .iter()
            .map(|line| {
                (
                    line.field("mode").to_owned(),
                    line.field("user_id").to_owned(),
                    line.field("idle_days").to_owned(),
                    line.field("action").to_owned(),
                )
            })
            .collect();
        let line = |h: Holder, days: &str, action: &str| {
            (
                "observe".to_owned(),
                h.user_id.to_string(),
                days.to_owned(),
                action.to_owned(),
            )
        };
        assert_eq!(
            logged,
            vec![
                line(long_gone, "30", "warn"),
                line(gone, "20", "hold"),
                line(fading, "8", "hold"),
            ],
            "one line per candidate, with the action enforce would take"
        );
        assert!(
            reported
                .iter()
                .all(|line| line.fields.values().all(|value| !value.contains('@'))),
            "a candidate line never carries an email: {reported:?}"
        );
        let summary = only(&lines, "strava seat reclaim pass");
        assert_eq!(summary.field("candidates"), "3");
        assert_eq!(summary.field("deficit"), "2");

        assert!(report.warned.is_empty() && report.reclaimed.is_empty());
        for h in [long_gone, gone, fading, active] {
            assert!(notifications_of(&resources, h).await.is_empty());
            assert!(warning_of(&resources, h).await.is_none());
            assert!(holds_strava_token(&resources, h).await);
        }
        assert!(stub.received().is_empty(), "observe revokes nothing");
    }

    /// Enforce warns the longest-idle athlete first, reclaims nobody until the
    /// lead has passed, then disconnects through the chokepoint: the grant is
    /// revoked at Strava with the stored refresh token, the token row and the
    /// warning go, and `provider.disconnected` names the seat reclaim.
    #[tokio::test]
    async fn enforce_warns_then_reclaims_only_after_the_lead() {
        let resources = resources().await;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        let first = holder(&resources, "first", t0, 30).await;
        let second = holder(&resources, "second", t0, 20).await;
        holder(&resources, "busy-a", t0, 1).await;
        holder(&resources, "busy-b", t0, 0).await;
        let (notifications, channel) = linked_notifications(&resources);
        let sweeper = reclaimer_with(&resources, &stub.url, Some(notifications));

        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(warned_users(&pass), vec![first.user_id]);
        assert!(pass.reclaimed.is_empty(), "nobody is reclaimed unwarned");
        assert_eq!(
            warning_of(&resources, first).await,
            Some((t0.timestamp_millis(), true)),
            "recorded as reached: it went out on the athlete's linked channel"
        );
        assert_eq!(
            channel.lock_sent(),
            vec![(
                first.user_id,
                "Your Strava connection will be released soon".to_owned()
            )]
        );
        let rows = notifications_of(&resources, first).await;
        assert_eq!(rows.len(), 1, "one warning notification");
        assert_eq!(rows[0].notification_type, "seat_release_warning");
        assert_eq!(
            rows[0].title,
            "Your Strava connection will be released soon"
        );
        assert_eq!(
            rows[0].body,
            "You haven't used Dravr in 30 day(s). Strava limits how many athletes we can connect, \
             so your Strava connection will be released in 3 day(s) to free the spot for another \
             athlete. Send a message or open the app to keep it; you can reconnect at any time."
        );
        assert_eq!(rows[0].data.as_ref().unwrap()["screen"], "connections");

        // An hour on, the lead is still running: the first waits, and the
        // second is warned because the shortfall is two seats.
        let pass = sweeper.tick(t0 + Duration::hours(1)).await.unwrap();
        assert_eq!(action_of(&pass, first), Some(ReclaimAction::AwaitLead));
        assert_eq!(warned_users(&pass), vec![second.user_id]);
        assert!(pass.reclaimed.is_empty());
        assert!(stub.received().is_empty());
        assert_eq!(
            notifications_of(&resources, first).await.len(),
            1,
            "a standing warning is not sent again"
        );

        // Past the first one's lead: reclaimed, one per pass.
        let (events, guard) = capture_notify();
        let pass = sweeper
            .tick(t0 + Duration::days(3) + Duration::minutes(30))
            .await
            .unwrap();
        drop(guard);
        assert_eq!(reclaimed_users(&pass), vec![first.user_id]);
        assert_eq!(pass.reclaimed[0].revocation, RevocationOutcome::Revoked);
        assert_eq!(
            action_of(&pass, second),
            Some(ReclaimAction::AwaitLead),
            "the second's lead runs from its own warning"
        );
        let revocations = stub.received();
        assert_eq!(revocations.len(), 1, "one revocation reached Strava");
        assert!(revocations[0].starts_with("POST "));
        assert!(revocations[0].contains(&format!("token={REFRESH_TOKEN}")));
        assert!(!holds_strava_token(&resources, first).await);
        assert!(warning_of(&resources, first).await.is_none());
        let event = only(&events, "provider.disconnected");
        assert_eq!(event.field("reason"), "seat_reclaim");
        assert_eq!(event.field("provider"), "strava");
        assert_eq!(event.field("user_id"), first.user_id.to_string());
        assert_eq!(event.field("tenant_id"), first.tenant_id.to_string());

        // One seat is free now, so one more is needed: the second goes once
        // its own lead is over, and then the pool is back at min_free_seats.
        let pass = sweeper
            .tick(t0 + Duration::days(3) + Duration::hours(2))
            .await
            .unwrap();
        assert_eq!(pass.free_seats, 1);
        assert_eq!(reclaimed_users(&pass), vec![second.user_id]);
        assert!(!holds_strava_token(&resources, second).await);

        let pass = sweeper
            .tick(t0 + Duration::days(3) + Duration::hours(3))
            .await
            .unwrap();
        assert_eq!(pass.free_seats, 2);
        assert_eq!(pass.deficit, 0);
        assert!(pass.warned.is_empty() && pass.reclaimed.is_empty());
    }

    impl LinkedChannel {
        fn lock_sent(&self) -> Vec<(Uuid, String)> {
            self.sent.lock().unwrap().clone()
        }
    }

    /// An athlete active after their warning keeps the seat even once the
    /// warning's lead is long over: the warning is withdrawn, nothing reaches
    /// Strava, and one still idle enough is warned afresh rather than
    /// disconnected on the stale warning. One who is no longer idle at all
    /// simply loses the warning.
    #[tokio::test]
    async fn activity_since_the_warning_cancels_the_reclaim() {
        let resources = resources().await;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        let returning = holder(&resources, "returning", t0, 30).await;
        let back_for_good = holder(&resources, "back-for-good", t0, 25).await;
        let pass_at = t0 + Duration::days(8) + Duration::hours(2);
        for label in ["busy-a", "busy-b"] {
            let busy = holder(&resources, label, t0, 0).await;
            set_last_active(&resources, busy, pass_at).await;
        }
        let sweeper = reclaimer(&resources, &stub.url);

        assert_eq!(
            warned_users(&sweeper.tick(t0).await.unwrap()),
            vec![returning.user_id]
        );
        assert_eq!(
            warned_users(&sweeper.tick(t0 + Duration::hours(1)).await.unwrap()),
            vec![back_for_good.user_id]
        );
        // Each comes back once, right after being warned, then goes quiet
        // again; only the second stays active up to the pass.
        set_last_active(&resources, returning, t0 + Duration::hours(1)).await;
        set_last_active(&resources, back_for_good, pass_at).await;

        let pass = sweeper.tick(pass_at).await.unwrap();
        assert!(
            pass.reclaimed.is_empty(),
            "active since the warning: the lead ran out, but the warning no longer stands"
        );
        assert_eq!(pass.warnings_cleared, 2);
        assert_eq!(action_of(&pass, back_for_good), None, "no longer idle");
        assert!(warning_of(&resources, back_for_good).await.is_none());
        assert_eq!(
            warned_users(&pass),
            vec![returning.user_id],
            "still idle past the warning threshold, so warned again"
        );
        assert_eq!(
            warning_of(&resources, returning).await,
            Some((pass_at.timestamp_millis(), true))
        );
        assert_eq!(notifications_of(&resources, returning).await.len(), 2);
        for kept in [returning, back_for_good] {
            assert!(holds_strava_token(&resources, kept).await);
        }
        assert!(stub.received().is_empty());
    }

    /// An athlete who uses Dravr only from an MCP client — a connector's
    /// `OAuth2` access token, or an API key — never logs in to the web app
    /// again. Every request the auth middleware authenticates is activity, so
    /// neither is idle, while the same athlete who did nothing would be.
    #[tokio::test]
    async fn mcp_connector_and_api_key_requests_count_as_activity() {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        let t0 = Utc::now();
        let connector = holder(&resources, "connector", t0, 30).await;
        let api_client = holder(&resources, "api-client", t0, 30).await;
        let idle_one = holder(&resources, "idle", t0, 30).await;

        let access_token = resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &resources.auth.jwks_manager,
                &connector.user_id,
                &["read".to_owned()],
                &[],
                Some(connector.tenant_id.to_string()),
            )
            .unwrap();
        let (api_key, api_key_value) = ApiKeyManager::new()
            .create_api_key(
                api_client.user_id,
                CreateApiKeyRequest {
                    name: "seat-reclaim".to_owned(),
                    description: None,
                    tier: ApiKeyTier::Starter,
                    rate_limit_requests: Some(1000),
                    expires_in_days: None,
                },
            )
            .unwrap();
        resources
            .common
            .repos
            .api_keys
            .create(&api_key)
            .await
            .unwrap();

        let middleware = &resources.auth.auth_middleware;
        // The connector's token is a delegated grant, so it authenticates on
        // the MCP path — the scoped entry point — where it is used.
        let by_connector = middleware
            .authenticate_scoped_request(Some(&format!("Bearer {access_token}")))
            .await
            .unwrap();
        assert_eq!(by_connector.user_id, connector.user_id);
        let by_key = middleware
            .authenticate_request(Some(&api_key_value))
            .await
            .unwrap();
        assert_eq!(by_key.user_id, api_client.user_id);

        let report = reclaimer(&resources, &stub.url)
            .tick(Utc::now())
            .await
            .unwrap();
        let candidates: Vec<Uuid> = report.candidates.iter().map(|c| c.user_id).collect();
        assert_eq!(
            candidates,
            vec![idle_one.user_id],
            "the connector and the API client are active; only the one who did nothing is idle"
        );
        for (athlete, via) in [
            (connector, "an OAuth2 connector token"),
            (api_client, "an API key"),
        ] {
            let seen = resources
                .common
                .repos
                .users
                .get_global(athlete.user_id)
                .await
                .unwrap()
                .unwrap()
                .last_active;
            assert!(
                Utc::now() - seen < Duration::minutes(1),
                "a request with {via} moved last_active to now, not {seen}"
            );
        }
    }

    /// A reclaim must free a seat the pressure count sees. A holder on a
    /// disabled pool app, on an app holding more athletes than its cap, or
    /// holding one app's seat through tokens in two tenants frees none, so
    /// however long idle they are never warned or reclaimed, and the pass
    /// spends its budget on the one athlete whose disconnect does free a seat.
    #[tokio::test]
    async fn only_a_holder_whose_disconnect_frees_a_counted_seat_is_acted_on() {
        const DISABLED: &str = "seat-reclaim-disabled-app";
        const OVER_CAP: &str = "seat-reclaim-over-cap-app";
        let resources = resources().await;
        let repos = &resources.common.repos;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        repos
            .oauth_tokens
            .upsert_strava_pool_app(DISABLED, "disabled-secret-with-thirty-chars", 3, None)
            .await
            .unwrap();
        repos
            .oauth_tokens
            .set_strava_pool_app_enabled(DISABLED, false)
            .await
            .unwrap();
        repos
            .oauth_tokens
            .upsert_strava_pool_app(OVER_CAP, "over-cap-secret-with-thirty-chars", 1, None)
            .await
            .unwrap();

        let disabled_a = holder_on(&resources, "disabled-a", t0, 60, Some(DISABLED)).await;
        let disabled_b = holder_on(&resources, "disabled-b", t0, 50, Some(DISABLED)).await;
        let over_cap_idle = holder_on(&resources, "over-cap", t0, 40, Some(OVER_CAP)).await;
        holder_on(&resources, "over-cap-busy", t0, 1, Some(OVER_CAP)).await;
        // One athlete, one env-app seat, two tenants' tokens.
        let two_tenants = holder(&resources, "two-tenants", t0, 45).await;
        let second_tenant = TenantId::generate();
        repos
            .tenants
            .create(&Tenant {
                id: second_tenant,
                name: "second tenant".to_owned(),
                slug: format!("second-{second_tenant}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: two_tenants.user_id,
                created_at: t0,
                updated_at: t0,
            })
            .await
            .unwrap();
        let two_tenants_again = Holder {
            user_id: two_tenants.user_id,
            tenant_id: second_tenant,
        };
        connect_strava(&resources, two_tenants_again, None, long_ago(t0)).await;
        let env_idle = holder(&resources, "env-idle", t0, 20).await;
        for label in ["env-busy-a", "env-busy-b"] {
            holder(&resources, label, t0, 0).await;
        }
        let sweeper = reclaimer(&resources, &stub.url);

        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(
            pass.free_seats, 0,
            "the env app is full and the enabled pool app is past its cap"
        );
        assert_eq!(pass.deficit, 2);
        for no_seat in [
            disabled_a,
            disabled_b,
            over_cap_idle,
            two_tenants,
            two_tenants_again,
        ] {
            assert_eq!(
                action_of(&pass, no_seat),
                Some(ReclaimAction::FreesNoSeat),
                "a candidate whose disconnect frees no counted seat"
            );
        }
        assert_eq!(action_of(&pass, env_idle), Some(ReclaimAction::Warn));
        assert_eq!(warned_users(&pass), vec![env_idle.user_id]);

        for hour in 1..=3 {
            let pass = sweeper.tick(t0 + Duration::hours(hour)).await.unwrap();
            assert!(
                pass.warned.is_empty(),
                "no one else's disconnect would free a seat"
            );
        }
        let pass = sweeper.tick(t0 + Duration::days(4)).await.unwrap();
        assert_eq!(reclaimed_users(&pass), vec![env_idle.user_id]);
        assert_eq!(pass.free_seats, 0, "counted before the pass's own reclaim");
        let pass = sweeper
            .tick(t0 + Duration::days(4) + Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(pass.free_seats, 1, "the reclaim freed a counted seat");
        assert!(pass.reclaimed.is_empty() && pass.warned.is_empty());
        assert_eq!(stub.received().len(), 1, "one revocation in all");
        for kept in [
            disabled_a,
            disabled_b,
            over_cap_idle,
            two_tenants,
            two_tenants_again,
        ] {
            assert!(holds_strava_token(&resources, kept).await);
            assert!(warning_of(&resources, kept).await.is_none());
        }
    }

    /// A warning is about one connection and goes stale. One sent before the
    /// athlete reconnected does not stand for the new connection, and one
    /// older than `idle_days` is sent again; neither leads to a disconnect
    /// without a fresh warning and a fresh lead.
    #[tokio::test]
    async fn a_warning_does_not_outlive_its_connection_or_idle_days() {
        let resources = resources().await;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let admin_id = admin(&resources.common.repos).await;
        set_policy_ok(&resources, &admin_id, &[(keys::MAX_PER_TICK_KEY, json!(2))]).await;
        let t0 = Utc::now();
        let reconnected = holder(&resources, "reconnected", t0, 30).await;
        let forgotten = holder(&resources, "forgotten", t0, 30).await;
        for label in ["busy-a", "busy-b"] {
            holder(&resources, label, t0, 0).await;
        }
        let sweeper = reclaimer(&resources, &stub.url);

        let pass = sweeper.tick(t0).await.unwrap();
        let mut warned = warned_users(&pass);
        warned.sort_unstable();
        let mut expected = vec![reconnected.user_id, forgotten.user_id];
        expected.sort_unstable();
        assert_eq!(warned, expected);

        // The first reconnects a day after the warning, through a path that
        // does not touch last_active.
        connect_strava(&resources, reconnected, None, t0 + Duration::days(1)).await;
        let pass = sweeper.tick(t0 + Duration::days(4)).await.unwrap();
        assert_eq!(
            reclaimed_users(&pass),
            vec![forgotten.user_id],
            "the untouched warning is due; the one before the reconnect is not"
        );
        assert_eq!(pass.warnings_cleared, 1);
        assert_eq!(
            warned_users(&pass),
            vec![reconnected.user_id],
            "the reconnected athlete is warned about the new connection"
        );
        assert!(holds_strava_token(&resources, reconnected).await);
        assert_eq!(stub.received().len(), 1);

        // Its fresh warning then sits past idle_days with the sweeper back
        // in observe, which reports it due and acts on nothing. Back in
        // enforce, the warning is too old to act on: it is sent again.
        set_policy_ok(
            &resources,
            &admin_id,
            &[(keys::MODE_KEY, json!(keys::MODE_OBSERVE))],
        )
        .await;
        let observed = sweeper.tick(t0 + Duration::days(8)).await.unwrap();
        assert_eq!(
            action_of(&observed, reconnected),
            Some(ReclaimAction::Reclaim)
        );
        assert!(observed.reclaimed.is_empty());
        set_policy_ok(
            &resources,
            &admin_id,
            &[(keys::MODE_KEY, json!(keys::MODE_ENFORCE))],
        )
        .await;
        let stale_at =
            t0 + Duration::days(4) + Duration::days(keys::DEFAULT_IDLE_DAYS) + Duration::hours(1);
        let pass = sweeper.tick(stale_at).await.unwrap();
        assert!(
            pass.reclaimed.is_empty(),
            "a stale warning justifies nothing"
        );
        assert_eq!(pass.warnings_cleared, 1);
        assert_eq!(warned_users(&pass), vec![reconnected.user_id]);
        assert_eq!(
            warning_of(&resources, reconnected).await,
            Some((stale_at.timestamp_millis(), true))
        );
        assert_eq!(notifications_of(&resources, reconnected).await.len(), 3);
        assert!(stub.received().is_empty());
    }

    /// Activity is read again right before each disconnect. An athlete who
    /// comes back while the pass is still disconnecting the one before them
    /// keeps the seat and loses the warning, although the pass planned their
    /// disconnect on activity read at its start.
    #[tokio::test]
    async fn an_athlete_back_during_the_pass_keeps_the_seat() {
        let resources = resources().await;
        enforce(&resources).await;
        let admin_id = admin(&resources.common.repos).await;
        set_policy_ok(&resources, &admin_id, &[(keys::MAX_PER_TICK_KEY, json!(2))]).await;
        let t0 = Utc::now();
        let gone = holder(&resources, "gone", t0, 40).await;
        let coming_back = holder(&resources, "coming-back", t0, 30).await;
        for label in ["busy-a", "busy-b"] {
            holder(&resources, label, t0, 0).await;
        }
        let mut stub = RevokeStub::start_bumping(&resources, coming_back).await;
        let sweeper = reclaimer(&resources, &stub.url);
        assert_eq!(sweeper.tick(t0).await.unwrap().warned.len(), 2);

        let pass = sweeper.tick(t0 + Duration::days(4)).await.unwrap();
        assert_eq!(action_of(&pass, gone), Some(ReclaimAction::Reclaim));
        assert_eq!(
            action_of(&pass, coming_back),
            Some(ReclaimAction::Reclaim),
            "planned on the activity read at the start of the pass"
        );
        assert_eq!(reclaimed_users(&pass), vec![gone.user_id]);
        assert_eq!(pass.reclaims_skipped, 1);
        assert_eq!(stub.received().len(), 1, "only the first reached Strava");
        assert!(holds_strava_token(&resources, coming_back).await);
        assert!(warning_of(&resources, coming_back).await.is_none());
    }

    /// No shortfall, nothing done; a shortfall bounds how many are warned and
    /// reclaimed, and `max_per_tick` bounds each pass.
    #[tokio::test]
    async fn min_free_seats_and_max_per_tick_bound_each_pass() {
        let resources = resources().await;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let admin_id = admin(&resources.common.repos).await;
        let t0 = Utc::now();
        let idlers = [
            holder(&resources, "idle-a", t0, 40).await,
            holder(&resources, "idle-b", t0, 30).await,
            holder(&resources, "idle-c", t0, 20).await,
            holder(&resources, "idle-d", t0, 15).await,
        ];
        let sweeper = reclaimer(&resources, &stub.url);

        set_policy_ok(
            &resources,
            &admin_id,
            &[(keys::MIN_FREE_SEATS_KEY, json!(0))],
        )
        .await;
        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(pass.deficit, 0);
        assert_eq!(pass.candidates.len(), 4, "every idle holder is reported");
        assert!(pass
            .candidates
            .iter()
            .all(|c| c.action == ReclaimAction::Hold));
        assert!(pass.warned.is_empty(), "no shortfall, no warning");

        set_policy_ok(
            &resources,
            &admin_id,
            &[
                (keys::MIN_FREE_SEATS_KEY, json!(3)),
                (keys::MAX_PER_TICK_KEY, json!(2)),
            ],
        )
        .await;
        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(pass.deficit, 3);
        assert_eq!(
            warned_users(&pass),
            vec![idlers[0].user_id, idlers[1].user_id],
            "two per pass, longest idle first"
        );
        let pass = sweeper.tick(t0 + Duration::hours(1)).await.unwrap();
        assert_eq!(
            warned_users(&pass),
            vec![idlers[2].user_id],
            "the third warning covers the three-seat shortfall; the fourth is never warned"
        );
        assert!(notifications_of(&resources, idlers[3]).await.is_empty());

        let pass = sweeper.tick(t0 + Duration::days(4)).await.unwrap();
        assert_eq!(
            reclaimed_users(&pass),
            vec![idlers[0].user_id, idlers[1].user_id],
            "max_per_tick disconnects"
        );
        let pass = sweeper
            .tick(t0 + Duration::days(4) + Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(reclaimed_users(&pass), vec![idlers[2].user_id]);
        assert_eq!(stub.received().len(), 3);
        assert!(holds_strava_token(&resources, idlers[3]).await);

        let pass = sweeper
            .tick(t0 + Duration::days(4) + Duration::hours(2))
            .await
            .unwrap();
        assert_eq!(pass.free_seats, 3);
        assert!(pass.reclaimed.is_empty() && pass.warned.is_empty());
    }

    /// A BYO-credentials athlete holds no shared seat and a dead grant already
    /// gave its seat back: however long idle, neither is a candidate, warned or
    /// touched.
    #[tokio::test]
    async fn byo_and_non_counting_holders_are_never_candidates() {
        let resources = resources().await;
        let repos = &resources.common.repos;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();

        let byo = holder(&resources, "byo", t0, 90).await;
        repos
            .oauth_tokens
            .store_user_oauth_app(
                byo.user_id,
                "strava",
                "byo-client",
                "byo-secret",
                "http://localhost/callback",
            )
            .await
            .unwrap();
        let dead = holder(&resources, "dead", t0, 90).await;
        repos
            .provider_connections
            .mark_needs_reauth(
                dead.user_id,
                dead.tenant_id,
                "strava",
                Some("invalid_grant"),
                Utc::now(),
            )
            .await
            .unwrap();
        let idler = holder(&resources, "idle", t0, 40).await;
        for label in ["busy-a", "busy-b", "busy-c"] {
            holder(&resources, label, t0, 1).await;
        }
        let sweeper = reclaimer(&resources, &stub.url);

        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(pass.free_seats, 0, "the four counting holders fill the cap");
        let candidates: Vec<Uuid> = pass.candidates.iter().map(|c| c.user_id).collect();
        assert_eq!(candidates, vec![idler.user_id]);
        assert_eq!(warned_users(&pass), vec![idler.user_id]);

        let pass = sweeper.tick(t0 + Duration::days(4)).await.unwrap();
        assert_eq!(reclaimed_users(&pass), vec![idler.user_id]);
        assert_eq!(stub.received().len(), 1);
        for untouched in [byo, dead] {
            assert!(holds_strava_token(&resources, untouched).await);
            assert!(notifications_of(&resources, untouched).await.is_empty());
            assert!(warning_of(&resources, untouched).await.is_none());
        }
    }

    /// An athlete with no push device and no linked chat channel — the web
    /// app only — is warned through the production notification service,
    /// which can only put the warning in their in-app list. That reached
    /// nobody: it is recorded as unreached, is not sent again every pass, and
    /// never leads to a disconnect, and the pass moves its budget to the next
    /// athlete instead of counting on it.
    #[tokio::test]
    async fn a_warning_that_reaches_nobody_never_leads_to_a_disconnect() {
        let resources = resources().await;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        let web_only = holder(&resources, "web-only", t0, 30).await;
        let next = holder(&resources, "next", t0, 20).await;
        for label in ["busy-a", "busy-b"] {
            holder(&resources, label, t0, 0).await;
        }
        let sweeper = reclaimer_with(
            &resources,
            &stub.url,
            resources.common.notification_service.clone(),
        );

        let pass = sweeper.tick(t0).await.unwrap();
        assert!(pass.warned.is_empty(), "nothing reached the athlete");
        let unreached: Vec<Uuid> = pass.warned_unreached.iter().map(|(u, _)| *u).collect();
        assert_eq!(unreached, vec![web_only.user_id]);
        assert_eq!(
            warning_of(&resources, web_only).await,
            Some((t0.timestamp_millis(), false))
        );
        assert_eq!(notifications_of(&resources, web_only).await.len(), 1);

        let pass = sweeper.tick(t0 + Duration::hours(1)).await.unwrap();
        assert_eq!(action_of(&pass, web_only), Some(ReclaimAction::Unreached));
        let unreached: Vec<Uuid> = pass.warned_unreached.iter().map(|(u, _)| *u).collect();
        assert_eq!(
            unreached,
            vec![next.user_id],
            "an unreached warning frees no seat to count on, so the next athlete is warned"
        );

        let pass = sweeper.tick(t0 + Duration::days(4)).await.unwrap();
        assert_eq!(action_of(&pass, web_only), Some(ReclaimAction::Unreached));
        assert_eq!(action_of(&pass, next), Some(ReclaimAction::Unreached));
        assert!(pass.reclaimed.is_empty());
        assert!(stub.received().is_empty());
        assert!(holds_strava_token(&resources, web_only).await);
        assert_eq!(
            notifications_of(&resources, web_only).await.len(),
            1,
            "recorded, so not repeated every pass"
        );
    }

    /// A warning the athlete's notification settings suppress reached nobody
    /// and left no row: nothing is recorded, so no disconnect can follow, and
    /// it is sent on the first pass the settings allow it.
    #[tokio::test]
    async fn a_suppressed_warning_is_not_recorded() {
        let resources = resources().await;
        let mut stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        let muted = holder(&resources, "muted", t0, 30).await;
        for label in ["busy-a", "busy-b", "busy-c"] {
            holder(&resources, label, t0, 0).await;
        }
        let (notifications, channel) = linked_notifications(&resources);
        let preference = |enabled| UpsertNotificationPreferenceParams {
            user_id: muted.user_id,
            tenant_id: CommTenantId(muted.tenant_id.as_uuid()),
            category: NotificationCategory::System.as_str().to_owned(),
            enabled,
            sub_preferences: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            timezone: None,
            max_per_day: None,
        };
        notifications
            .upsert_notification_preference(&preference(false))
            .await
            .unwrap();
        let sweeper = reclaimer_with(&resources, &stub.url, Some(Arc::clone(&notifications)));

        for hours in [0, 1, 96] {
            let pass = sweeper.tick(t0 + Duration::hours(hours)).await.unwrap();
            assert_eq!(action_of(&pass, muted), Some(ReclaimAction::Warn));
            assert!(pass.warned.is_empty() && pass.warned_unreached.is_empty());
            assert!(pass.reclaimed.is_empty());
        }
        assert!(warning_of(&resources, muted).await.is_none());
        assert!(notifications_of(&resources, muted).await.is_empty());
        assert!(channel.lock_sent().is_empty());
        assert!(stub.received().is_empty());

        notifications
            .upsert_notification_preference(&preference(true))
            .await
            .unwrap();
        let resumed_at = t0 + Duration::hours(97);
        let pass = sweeper.tick(resumed_at).await.unwrap();
        assert_eq!(warned_users(&pass), vec![muted.user_id]);
        assert_eq!(
            warning_of(&resources, muted).await,
            Some((resumed_at.timestamp_millis(), true))
        );
    }

    /// `config set` reaches the next pass with no restart, `config get` reads
    /// the value back with its source, and a write that breaks the policy's
    /// ranges or its ordering is refused and stores nothing.
    #[tokio::test]
    async fn config_writes_change_the_policy_on_the_next_pass() {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        let admin_id = admin(&resources.common.repos).await;
        let t0 = Utc::now();
        let nine_days = holder(&resources, "nine-days", t0, 9).await;
        let sweeper = reclaimer(&resources, &stub.url);

        let pass = sweeper.tick(t0).await.unwrap();
        assert!(
            action_of(&pass, nine_days).is_some(),
            "idle 9 days is past the default 10 - 3"
        );

        set_policy_ok(&resources, &admin_id, &[(keys::IDLE_DAYS_KEY, json!(14))]).await;
        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(pass.policy.idle_days, 14);
        assert_eq!(
            action_of(&pass, nine_days),
            None,
            "idle 9 days is short of 14 - 3"
        );

        // What `pierre-cli config get strava_seat_reclaim.idle_days` prints.
        let catalog = resources
            .agent
            .admin_config
            .as_ref()
            .unwrap()
            .get_catalog(ConfigLookupScope::global())
            .await
            .unwrap();
        let category = catalog
            .categories
            .iter()
            .find(|c| c.name == keys::CATEGORY)
            .expect("the category row lists the policy");
        let listed: Vec<&str> = {
            let mut keys_listed: Vec<&str> =
                category.parameters.iter().map(|p| p.key.as_str()).collect();
            keys_listed.sort_unstable();
            keys_listed
        };
        assert_eq!(
            listed,
            vec![
                keys::IDLE_DAYS_KEY,
                keys::MAX_PER_TICK_KEY,
                keys::MIN_FREE_SEATS_KEY,
                keys::MODE_KEY,
                keys::WARN_LEAD_DAYS_KEY,
            ]
        );
        let idle_days = category
            .parameters
            .iter()
            .find(|p| p.key == keys::IDLE_DAYS_KEY)
            .unwrap();
        assert_eq!(idle_days.current_value, json!(14));
        assert_eq!(idle_days.value_source, "global");
        assert_eq!(idle_days.default_value, json!(10));
        let mode = category
            .parameters
            .iter()
            .find(|p| p.key == keys::MODE_KEY)
            .unwrap();
        assert_eq!(mode.current_value, json!("enforce"));
        assert_eq!(mode.value_source, "default");

        // A lead that no longer sits below idle_days is refused, naming both.
        let refused = set_policy(
            &resources,
            &admin_id,
            &[(keys::WARN_LEAD_DAYS_KEY, json!(14))],
        )
        .await;
        assert!(!refused.success);
        assert_eq!(refused.validation_errors.len(), 1);
        assert_eq!(
            refused.validation_errors[0].message,
            "strava_seat_reclaim.warn_lead_days (14) must stay below strava_seat_reclaim.idle_days (14)"
        );
        // So is lowering idle_days under the stored lead, and a value outside
        // the declared range.
        let refused = set_policy(&resources, &admin_id, &[(keys::IDLE_DAYS_KEY, json!(3))]).await;
        assert!(
            !refused.success,
            "idle_days 3 is not above warn_lead_days 3"
        );
        let refused = set_policy(&resources, &admin_id, &[(keys::IDLE_DAYS_KEY, json!(0))]).await;
        assert!(!refused.success, "idle_days 0 is out of range");
        let refused = set_policy(
            &resources,
            &admin_id,
            &[(keys::MAX_PER_TICK_KEY, json!(51))],
        )
        .await;
        assert!(!refused.success, "max_per_tick 51 is out of range");
        assert_eq!(sweeper.tick(t0).await.unwrap().policy.idle_days, 14);

        // Both keys in one write are checked against each other.
        set_policy_ok(
            &resources,
            &admin_id,
            &[
                (keys::IDLE_DAYS_KEY, json!(20)),
                (keys::WARN_LEAD_DAYS_KEY, json!(5)),
            ],
        )
        .await;
        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!((pass.policy.idle_days, pass.policy.warn_lead_days), (20, 5));

        // Off stops the sweeper looking at all.
        set_policy_ok(
            &resources,
            &admin_id,
            &[(keys::MODE_KEY, json!(keys::MODE_OFF))],
        )
        .await;
        let pass = sweeper.tick(t0).await.unwrap();
        assert_eq!(pass.policy.mode, ReclaimMode::Off);
        assert!(pass.candidates.is_empty());
    }

    /// `pierre-cli config reset` is held to the same ordering as `config set`:
    /// resetting `idle_days` alone under a lead that would then reach it is
    /// refused and leaves both stored values, so the sweeper keeps running;
    /// resetting the whole category puts both back to their defaults.
    #[tokio::test]
    async fn a_reset_cannot_leave_the_lead_at_or_above_idle_days() {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        let admin_id = admin(&resources.common.repos).await;
        let service = resources.agent.admin_config.as_ref().unwrap();
        let sweeper = reclaimer(&resources, &stub.url);
        set_policy_ok(
            &resources,
            &admin_id,
            &[
                (keys::IDLE_DAYS_KEY, json!(30)),
                (keys::WARN_LEAD_DAYS_KEY, json!(20)),
            ],
        )
        .await;
        let reset = |keys: Option<Vec<String>>| ResetConfigRequest {
            category: Some(keys::CATEGORY.to_owned()),
            keys,
            reason: Some("carnet#505 test".to_owned()),
        };

        let error = service
            .reset_config(
                &reset(Some(vec![keys::IDLE_DAYS_KEY.to_owned()])),
                context(&admin_id),
            )
            .await
            .expect_err("idle_days back to 10 under a lead of 20 must be refused");
        assert!(
            error.to_string().ends_with(
                "strava_seat_reclaim.warn_lead_days (20) must stay below \
                 strava_seat_reclaim.idle_days (10) after this reset; reset both keys together, \
                 or set the one you keep first"
            ),
            "{error}"
        );
        let pass = sweeper.tick(Utc::now()).await.unwrap();
        assert_eq!(
            (pass.policy.idle_days, pass.policy.warn_lead_days),
            (30, 20),
            "the refused reset stored nothing, and the pass still runs"
        );

        // Resetting the lead alone orders the pair, so it is accepted.
        let response = service
            .reset_config(
                &reset(Some(vec![keys::WARN_LEAD_DAYS_KEY.to_owned()])),
                context(&admin_id),
            )
            .await
            .unwrap();
        assert_eq!(
            response.reset_keys,
            vec![keys::WARN_LEAD_DAYS_KEY.to_owned()]
        );
        let pass = sweeper.tick(Utc::now()).await.unwrap();
        assert_eq!((pass.policy.idle_days, pass.policy.warn_lead_days), (30, 3));

        set_policy_ok(
            &resources,
            &admin_id,
            &[(keys::WARN_LEAD_DAYS_KEY, json!(20))],
        )
        .await;
        let response = service
            .reset_config(&reset(None), context(&admin_id))
            .await
            .unwrap();
        assert_eq!(response.reset_count, 2);
        let pass = sweeper.tick(Utc::now()).await.unwrap();
        assert_eq!(
            (pass.policy.idle_days, pass.policy.warn_lead_days),
            (keys::DEFAULT_IDLE_DAYS, keys::DEFAULT_WARN_LEAD_DAYS)
        );
    }

    /// Every stored value is checked again at each pass, so one the catalog
    /// would have refused — written straight through the repository, as a
    /// hand edit would be — stops the pass instead of steering it: a lead at
    /// `idle_days`, a blast-radius cap far past its ceiling, a mode that is not
    /// one of the three.
    #[tokio::test]
    async fn an_invalid_stored_policy_refuses_the_pass() {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        let sweeper = reclaimer(&resources, &stub.url);
        let refusal = || async { sweeper.tick(Utc::now()).await.unwrap_err().to_string() };

        hand_edit(
            &resources,
            keys::WARN_LEAD_DAYS_KEY,
            json!(10),
            ConfigDataType::Integer,
        )
        .await;
        assert!(
            refusal().await.contains(
                "strava_seat_reclaim.warn_lead_days (10) must stay below strava_seat_reclaim.idle_days (10)"
            ),
            "a lead equal to idle_days"
        );
        hand_edit(
            &resources,
            keys::WARN_LEAD_DAYS_KEY,
            json!(3),
            ConfigDataType::Integer,
        )
        .await;

        hand_edit(
            &resources,
            keys::MAX_PER_TICK_KEY,
            json!(1000),
            ConfigDataType::Integer,
        )
        .await;
        assert!(
            refusal()
                .await
                .contains("strava_seat_reclaim.max_per_tick is 1000, outside 1..=50"),
            "a cap past its ceiling"
        );
        hand_edit(
            &resources,
            keys::MAX_PER_TICK_KEY,
            json!(1),
            ConfigDataType::Integer,
        )
        .await;

        hand_edit(
            &resources,
            keys::MIN_FREE_SEATS_KEY,
            json!(1000),
            ConfigDataType::Integer,
        )
        .await;
        assert!(refusal()
            .await
            .contains("strava_seat_reclaim.min_free_seats is 1000, outside 0..=100"));
        hand_edit(
            &resources,
            keys::MIN_FREE_SEATS_KEY,
            json!(2),
            ConfigDataType::Integer,
        )
        .await;

        hand_edit(
            &resources,
            keys::MODE_KEY,
            json!("enforced"),
            ConfigDataType::Enum,
        )
        .await;
        assert!(
            refusal().await.contains(
                "strava_seat_reclaim.mode is \"enforced\", not one of off, observe, enforce"
            ),
            "an unknown mode"
        );
        hand_edit(
            &resources,
            keys::MODE_KEY,
            json!("observe"),
            ConfigDataType::Enum,
        )
        .await;
        assert_eq!(
            sweeper.tick(Utc::now()).await.unwrap().policy.mode,
            ReclaimMode::Observe,
            "every value back in range, the pass runs"
        );
    }

    /// The warning reads in the athlete's own language, and a warning emits no
    /// disconnect event.
    #[tokio::test]
    async fn the_warning_reads_in_the_athletes_language() {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        let french = seed_user(&resources.common.repos, "francais", t0, idle(12), "fr").await;
        connect_strava(&resources, french, None, long_ago(t0)).await;
        for label in ["b", "c", "d"] {
            holder(&resources, label, t0, 0).await;
        }

        let (events, guard) = capture_notify();
        let pass = reclaimer(&resources, &stub.url).tick(t0).await.unwrap();
        drop(guard);
        assert_eq!(warned_users(&pass), vec![french.user_id]);
        assert!(
            named(&events, "provider.disconnected").is_empty(),
            "a warning disconnects nothing"
        );
        let rows = notifications_of(&resources, french).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].title,
            "Ta connexion à Strava va bientôt être libérée"
        );
        assert_eq!(
            rows[0].body,
            "Tu n'as pas utilisé Dravr depuis 12 jour(s). Strava limite le nombre d'athlètes que \
             nous pouvons connecter : ta connexion à Strava sera libérée dans 3 jour(s) pour \
             laisser la place à un autre athlète. Envoie un message ou ouvre l'app pour la \
             garder ; tu pourras te reconnecter à tout moment."
        );
    }

    /// The sweeper the boot wiring starts runs on the worker ledger: once the
    /// ledger says it is due, a pass runs on its own and warns.
    #[tokio::test]
    async fn the_started_sweeper_runs_its_pass_when_the_ledger_says_it_is_due() {
        let resources = resources().await;
        let stub = RevokeStub::start().await;
        enforce(&resources).await;
        let t0 = Utc::now();
        let idle_one = holder(&resources, "idle", t0, 30).await;
        for label in ["busy-a", "busy-b", "busy-c"] {
            holder(&resources, label, t0, 0).await;
        }
        let ledger = Arc::clone(&resources.common.repos.worker_runs);
        // Last run a period ago, less two seconds: due two seconds from now.
        let period_ms = i64::try_from(TICK_INTERVAL.as_millis()).unwrap();
        let seeded_last_run = Utc::now().timestamp_millis() - period_ms + 2_000;
        ledger
            .finish_worker_run(WORKER_NAME, seeded_last_run)
            .await
            .unwrap();

        start_strava_seat_reclaimer(
            Arc::new(reclaimer(&resources, &stub.url)),
            Arc::clone(&ledger),
        );
        let mut finished = false;
        for _ in 0..150 {
            let run = ledger.get_worker_run(WORKER_NAME).await.unwrap().unwrap();
            if run.last_run_at_ms > seeded_last_run {
                finished = true;
                break;
            }
            sleep(StdDuration::from_millis(100)).await;
        }
        assert!(
            finished,
            "the started sweeper finished a pass within fifteen seconds"
        );
        let (warned_at_ms, reached) = warning_of(&resources, idle_one)
            .await
            .expect("the pass warned the idle athlete");
        assert!(reached);
        assert!(warned_at_ms >= t0.timestamp_millis());
    }
}
