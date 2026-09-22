// ABOUTME: Verifies Strava pool seat accounting and app selection — the counts the recommender and admin listing read
// ABOUTME: Pool caps add capacity, a dead grant frees its seat, a client-side failure does not, a reconnect keeps its app
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::Utc;
use common::create_test_server_resources;
use pierre_auth::config::oauth::{get_oauth_config, strava_oauth_seat_cap};
use pierre_auth::strava_pool::{select_strava_app, strava_seat_summary, SelectedStravaApp};
use pierre_core::models::{ConnectionType, TenantId, User, UserOAuthToken};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A 30-character pool app secret; the selection hands it back decrypted.
const POOL_SECRET: &str = "poolsecretvaluewithlength30chr";

/// A pool app's `seat_cap` must add to the capacity the recommender reads.
///
/// `compute_providers_status` reads `strava_seat_summary` to decide whether to
/// keep offering Strava OAuth (`recommended_backend: oauth`) or fall back to the
/// Sciotte mirror. Adding a pool app must raise the total capacity by its
/// `seat_cap` without changing how many seats are already used — that is the
/// whole point of the pool.
#[tokio::test]
async fn pool_app_seat_cap_adds_to_total_capacity() {
    let resources = create_test_server_resources().await.unwrap();
    let repo = &resources.common.repos.oauth_tokens;

    let before = strava_seat_summary(repo.as_ref())
        .await
        .expect("seat summary");

    repo.upsert_strava_pool_app("777777", "poolsecretvaluewithlength30chr", 5, Some("app-2"))
        .await
        .expect("add pool app");

    let after = strava_seat_summary(repo.as_ref())
        .await
        .expect("seat summary");

    assert_eq!(
        after.total,
        before.total + 5,
        "an enabled pool app adds its seat_cap to the total pool capacity"
    );
    assert_eq!(
        after.used, before.used,
        "adding an empty pool app does not change occupied seats"
    );
    assert_eq!(
        after.left(),
        after.total - after.used,
        "left = total - used"
    );
    assert!(
        after.left() >= before.left() + 5,
        "the extra capacity is available as free seats"
    );

    // Disabling the app removes its capacity again.
    repo.set_strava_pool_app_enabled("777777", false)
        .await
        .unwrap();
    let disabled = strava_seat_summary(repo.as_ref()).await.unwrap();
    assert_eq!(
        disabled.total, before.total,
        "a disabled pool app contributes no capacity"
    );
}

/// A distinct athlete; `user_oauth_tokens.user_id` references `users` on Postgres.
async fn fresh_athlete(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("strava-seat-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Strava Seat Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

/// Leave the state a completed OAuth callback leaves: a Strava token issued by
/// `app` (`None` = the env-default app) and an active connection row.
async fn connect(repos: &RepositoryRegistry, athlete: Uuid, tenant: TenantId, app: Option<&str>) {
    let token = UserOAuthToken::new(
        athlete,
        tenant.to_string(),
        "strava".to_owned(),
        "acc".to_owned(),
        Some("ref".to_owned()),
        Some(Utc::now() + chrono::Duration::hours(6)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(app.map(str::to_owned));
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(athlete, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
}

/// Write `revoked` onto a Strava connection. It is one of the statuses
/// `ConnectionStatus::requires_reauth` names, so the seat count honours it,
/// but no repository method writes it, so the test sets the column itself.
async fn revoke_connection(database: &Database, athlete: Uuid, tenant: TenantId) {
    const SQL: &str = "UPDATE provider_connections SET status = 'revoked' \
                       WHERE user_id = $1 AND tenant_id = $2 AND provider = 'strava'";
    let affected = match database {
        Database::SQLite(db) => sqlx::query(SQL)
            .bind(athlete.to_string())
            .bind(tenant.to_string())
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query(SQL)
            .bind(athlete.to_string())
            .bind(tenant.to_string())
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(
        affected, 1,
        "exactly the athlete's Strava connection is revoked"
    );
}

/// Usage by app, sorted so the assertion does not depend on row order.
async fn usage_by_app(
    repos: &RepositoryRegistry,
    excluded: Option<Uuid>,
) -> Vec<(Option<String>, u32)> {
    let mut usage = repos
        .oauth_tokens
        .count_strava_seat_usage_by_app(excluded)
        .await
        .unwrap();
    usage.sort();
    usage
}

/// A token whose grant Strava rejected (`needs_reauth` over `invalid_grant`)
/// or whose access was revoked holds no seat; a live token and a token with no
/// connection row still do. Both readers of the count — the per-app usage the
/// authorize path fills from and the seat summary the recommender and the
/// admin listing read — agree, and a reconnect takes the seat back.
#[tokio::test]
async fn a_token_whose_connection_needs_reauth_or_is_revoked_holds_no_seat() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let tenant = TenantId::generate();
    repos
        .oauth_tokens
        .upsert_strava_pool_app(
            "201455",
            "poolsecretvaluewithlength30chr",
            10,
            Some("app-2"),
        )
        .await
        .unwrap();

    let live_env = fresh_athlete(repos).await;
    let dead_env = fresh_athlete(repos).await;
    let revoked_env = fresh_athlete(repos).await;
    let unregistered_env = fresh_athlete(repos).await;
    let live_pool = fresh_athlete(repos).await;
    let dead_pool = fresh_athlete(repos).await;

    connect(repos, live_env, tenant, None).await;
    connect(repos, dead_env, tenant, None).await;
    connect(repos, revoked_env, tenant, None).await;
    connect(repos, live_pool, tenant, Some("201455")).await;
    connect(repos, dead_pool, tenant, Some("201455")).await;
    // A token with no connection row at all still occupies its seat.
    let orphan = UserOAuthToken::new(
        unregistered_env,
        tenant.to_string(),
        "strava".to_owned(),
        "acc".to_owned(),
        Some("ref".to_owned()),
        Some(Utc::now() + chrono::Duration::hours(6)),
        Some("read".to_owned()),
    );
    repos.oauth_tokens.upsert_token(&orphan).await.unwrap();

    assert_eq!(
        usage_by_app(repos, None).await,
        vec![(None, 4), (Some("201455".to_owned()), 2)],
        "every token holds a seat while its connection is active"
    );

    for athlete in [dead_env, dead_pool] {
        repos
            .provider_connections
            .mark_needs_reauth(athlete, tenant, "strava", Some("invalid_grant"), Utc::now())
            .await
            .unwrap();
    }
    revoke_connection(&resources.agent.database, revoked_env, tenant).await;

    assert_eq!(
        usage_by_app(repos, None).await,
        vec![(None, 2), (Some("201455".to_owned()), 1)],
        "needs_reauth and revoked tokens free their seats; live and unregistered ones keep theirs"
    );
    let summary = strava_seat_summary(repos.oauth_tokens.as_ref())
        .await
        .unwrap();
    assert_eq!(
        summary.used, 3,
        "the seat summary agrees with the per-app usage"
    );

    assert_eq!(
        usage_by_app(repos, Some(live_env)).await,
        vec![(None, 1), (Some("201455".to_owned()), 1)],
        "an excluded athlete leaves every bucket"
    );
    assert_eq!(
        usage_by_app(repos, Some(dead_env)).await,
        vec![(None, 2), (Some("201455".to_owned()), 1)],
        "excluding an athlete who holds no seat changes nothing"
    );

    repos
        .provider_connections
        .mark_active(dead_env, tenant, "strava")
        .await
        .unwrap();
    assert_eq!(
        usage_by_app(repos, None).await,
        vec![(None, 3), (Some("201455".to_owned()), 1)],
        "a re-armed connection takes its seat back"
    );
}

/// A refresh refused over our own client credentials flips the connection to
/// `needs_reauth` too, but the athlete's grant is still authorized at Strava,
/// which still counts them: the token keeps its seat. Every other reason a
/// connection needs re-authorizing — a dead grant, a bare 401 — leaves no
/// grant this platform can use, and frees it.
#[tokio::test]
async fn a_needs_reauth_over_our_own_client_credentials_keeps_its_seat() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let tenant = TenantId::generate();
    repos
        .oauth_tokens
        .upsert_strava_pool_app("201456", POOL_SECRET, 10, Some("rotated"))
        .await
        .unwrap();

    let mut athletes = Vec::new();
    for reason in [
        "invalid_client",
        "unauthorized_client",
        "unauthorized",
        "http 401",
        "invalid_grant",
        "invalid_request",
    ] {
        let athlete = fresh_athlete(repos).await;
        connect(repos, athlete, tenant, Some("201456")).await;
        repos
            .provider_connections
            .mark_needs_reauth(athlete, tenant, "strava", Some(reason), Utc::now())
            .await
            .unwrap();
        athletes.push(athlete);
    }

    assert_eq!(
        usage_by_app(repos, None).await,
        vec![(Some("201456".to_owned()), 2)],
        "the two client-credential failures hold their seats; the other four do not"
    );
    let holders = repos.oauth_tokens.list_strava_seat_holders().await.unwrap();
    let counting: Vec<bool> = athletes
        .iter()
        .map(|athlete| {
            holders
                .iter()
                .find(|h| h.user_id == *athlete)
                .expect("every athlete is listed")
                .counts_as_seat
        })
        .collect();
    assert_eq!(
        counting,
        vec![true, true, false, false, false, false],
        "the listing applies the same rule per athlete"
    );
}

/// The env-default app's `client_id`, resolved as the authorize path resolves it.
fn env_client_id() -> String {
    get_oauth_config("strava")
        .client_id
        .expect("the test harness configures the env Strava app")
}

/// Run the authorize-time selection for `athlete`.
async fn select(repos: &RepositoryRegistry, athlete: Uuid, tenant: TenantId) -> SelectedStravaApp {
    select_strava_app(repos.oauth_tokens.as_ref(), athlete, tenant)
        .await
        .expect("a seat remains somewhere")
}

/// Connect `count` fresh athletes on `app` and return them.
async fn fill(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    app: Option<&str>,
    count: u32,
) -> Vec<Uuid> {
    let mut athletes = Vec::new();
    for _ in 0..count {
        let athlete = fresh_athlete(repos).await;
        connect(repos, athlete, tenant, app).await;
        athletes.push(athlete);
    }
    athletes
}

/// An athlete whose live token the env app issued reconnects on the env app
/// even while it reads full: the athlete's own seat is one of the ones filling
/// it. Anyone new is sent to the pool at the same moment, which is what shows
/// the env app really is at its cap.
#[tokio::test]
async fn a_live_athlete_reconnects_on_the_env_app_while_it_reads_full() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let tenant = TenantId::generate();
    let cap = strava_oauth_seat_cap();
    repos
        .oauth_tokens
        .upsert_strava_pool_app("900001", POOL_SECRET, 5, Some("pool-a"))
        .await
        .unwrap();

    let athlete = fresh_athlete(repos).await;
    connect(repos, athlete, tenant, None).await;
    fill(repos, tenant, None, cap - 1).await;
    assert_eq!(
        usage_by_app(repos, None).await,
        vec![(None, cap)],
        "the env app is at its cap, the athlete included"
    );

    let reconnect = select(repos, athlete, tenant).await;
    assert_eq!(
        reconnect.attribution, None,
        "the reconnect stays on the env app"
    );
    assert_eq!(reconnect.client_id, env_client_id());

    let newcomer = fresh_athlete(repos).await;
    let first_connect = select(repos, newcomer, tenant).await;
    assert_eq!(first_connect.attribution.as_deref(), Some("900001"));
    assert_eq!(first_connect.client_id, "900001");
    assert_eq!(first_connect.client_secret, POOL_SECRET);
}

/// An athlete whose env-app grant died holds no seat, so their reconnect is
/// scored like anyone else's: the pool while the env app is full counting
/// everyone else, and back on the env app once another athlete's seat frees.
/// Landing on the pool is a switch of apps, which the callback completes by
/// revoking the env-app grant once the pool token is stored; that half runs
/// through the real callback in `strava_app_switch_test`.
#[tokio::test]
async fn a_dead_athlete_takes_the_pool_while_the_env_app_is_full_and_returns_when_it_frees() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let tenant = TenantId::generate();
    let cap = strava_oauth_seat_cap();
    repos
        .oauth_tokens
        .upsert_strava_pool_app("900002", POOL_SECRET, 5, Some("pool-b"))
        .await
        .unwrap();

    let athlete = fresh_athlete(repos).await;
    connect(repos, athlete, tenant, None).await;
    repos
        .provider_connections
        .mark_needs_reauth(athlete, tenant, "strava", Some("invalid_grant"), Utc::now())
        .await
        .unwrap();
    let others = fill(repos, tenant, None, cap).await;
    assert_eq!(
        usage_by_app(repos, None).await,
        vec![(None, cap)],
        "the env app is full with other athletes; the dead token is not among its seats"
    );

    let while_full = select(repos, athlete, tenant).await;
    assert_eq!(
        while_full.attribution.as_deref(),
        Some("900002"),
        "no env seat is free for the athlete, so the pool takes them"
    );
    assert_eq!(while_full.client_id, "900002");

    repos
        .provider_connections
        .mark_needs_reauth(
            others[0],
            tenant,
            "strava",
            Some("invalid_grant"),
            Utc::now(),
        )
        .await
        .unwrap();
    let once_freed = select(repos, athlete, tenant).await;
    assert_eq!(
        once_freed.attribution, None,
        "a seat freed on the env app brings the athlete back to it"
    );
    assert_eq!(once_freed.client_id, env_client_id());
}

/// An athlete with no token takes the fill order: the env app while it has
/// room, then the first enabled pool app with a free seat, then nothing.
#[tokio::test]
async fn a_new_athlete_fills_the_env_app_then_the_pool() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let tenant = TenantId::generate();
    let cap = strava_oauth_seat_cap();
    repos
        .oauth_tokens
        .upsert_strava_pool_app("900003", POOL_SECRET, 1, Some("pool-c"))
        .await
        .unwrap();
    let newcomer = fresh_athlete(repos).await;

    let empty = select(repos, newcomer, tenant).await;
    assert_eq!(
        empty.attribution, None,
        "an empty env app takes the first athlete"
    );
    assert_eq!(empty.client_id, env_client_id());

    fill(repos, tenant, None, cap).await;
    let env_full = select(repos, newcomer, tenant).await;
    assert_eq!(env_full.attribution.as_deref(), Some("900003"));
    assert_eq!(env_full.client_secret, POOL_SECRET);

    fill(repos, tenant, Some("900003"), 1).await;
    let err = select_strava_app(repos.oauth_tokens.as_ref(), newcomer, tenant)
        .await
        .expect_err("every app is at its cap");
    assert_eq!(
        err.message,
        "No Strava OAuth seats available across the env app and pool"
    );
}

/// An athlete whose token a pool app issued reconnects on that app, even while
/// the env app has room, as long as the app is enabled. A live grant keeps its
/// app even when it reads full counting everyone else, since Strava already
/// counts the athlete there; a dead one needs a seat like anyone new. A
/// disabled app, or a full one for a dead grant, hands them to the fill order.
#[tokio::test]
async fn a_pool_athlete_reconnects_on_its_pool_app_while_it_is_enabled_and_has_room() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let tenant = TenantId::generate();
    repos
        .oauth_tokens
        .upsert_strava_pool_app("900004", POOL_SECRET, 2, Some("pool-d"))
        .await
        .unwrap();

    let athlete = fresh_athlete(repos).await;
    connect(repos, athlete, tenant, Some("900004")).await;
    fill(repos, tenant, Some("900004"), 1).await;

    let reconnect = select(repos, athlete, tenant).await;
    assert_eq!(
        reconnect.attribution.as_deref(),
        Some("900004"),
        "the athlete's own pool app wins over an env app with room"
    );
    assert_eq!(reconnect.client_secret, POOL_SECRET);

    let newcomer = fresh_athlete(repos).await;
    assert_eq!(
        select(repos, newcomer, tenant).await.attribution,
        None,
        "anyone new still fills the env app first"
    );

    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("900004", false)
        .await
        .unwrap();
    assert_eq!(
        select(repos, athlete, tenant).await.attribution,
        None,
        "a disabled pool app is not reconnected on"
    );

    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("900004", true)
        .await
        .unwrap();
    fill(repos, tenant, Some("900004"), 1).await;
    assert_eq!(
        select(repos, athlete, tenant).await.attribution.as_deref(),
        Some("900004"),
        "a live grant keeps its app while it reads full: the athlete is already counted there"
    );

    repos
        .provider_connections
        .mark_needs_reauth(athlete, tenant, "strava", Some("invalid_grant"), Utc::now())
        .await
        .unwrap();
    assert_eq!(
        select(repos, athlete, tenant).await.attribution,
        None,
        "a dead grant on a pool app full counting everyone else is handed to the fill order"
    );
}
