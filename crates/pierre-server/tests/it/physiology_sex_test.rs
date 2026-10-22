// ABOUTME: The athlete's physiological sex: stored on the profile, stated with set_physiology, filled by a provider import
// ABOUTME: An athlete-set value always wins over a provider's, and the equations read it (Keytel, the LT1 pace cohort)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use dravr_cageux::nutrition_calculator::Gender;
use dravr_cageux::zones::AerobicThresholdPace;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{
    ConnectionType, PhysiologicalSex, SportType, TenantId, UserOAuthToken,
    UserPhysiologicalProfile, API_KEY_TOKEN_TYPE,
};
use pierre_database::repositories::UserPhysiologicalProfileRepository;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::tools::implementations::endurance_export::ExportDossierTool;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_test_support::db::create_test_db_url;
use pierre_tool_runtime::implementations::athlete_stats::GetAthleteTool;
use pierre_tool_runtime::implementations::physiology::SetPhysiologyTool;
use pierre_tool_runtime::implementations::physiology_import::ImportProviderThresholdsTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::{json, Value};
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user_with_plan};

/// A loopback Intervals.icu that answers the athlete profile with `sex`,
/// and sport settings that hold no thresholds.
async fn spawn_intervals(sex: Option<&'static str>) -> String {
    let app = Router::new()
        .route(
            "/api/v1/athlete/{athlete}",
            get(move |Path(athlete): Path<String>| async move {
                let mut body = json!({ "id": athlete, "name": "Test Athlete" });
                if let Some(sex) = sex {
                    body["sex"] = json!(sex);
                }
                (StatusCode::OK, Json(body))
            }),
        )
        .route(
            "/api/v1/athlete/{athlete}/sport-settings",
            get(|| async { (StatusCode::OK, Json(json!([]))) }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

struct World {
    res: Arc<ServerContext>,
    runtime: Arc<dyn ToolRuntime>,
}

impl World {
    async fn new(reported_sex: Option<&'static str>) -> Self {
        let base = create_test_server_resources().await.unwrap();
        let mut config = default_config();
        config.api_base_url = spawn_intervals(reported_sex).await;
        let mut registry = ProviderRegistry::new();
        registry.set_default_config(INTERVALS_ICU, config);
        let mut context = (*base).clone();
        context.fitness.provider_registry = Arc::new(registry);
        let res = Arc::new(context);
        let runtime: Arc<dyn ToolRuntime> = res.clone();
        Self { res, runtime }
    }

    async fn person(&self) -> (Uuid, TenantId) {
        let email = format!("sex-{}@physiology.test", Uuid::new_v4());
        let (id, _, tenant) =
            create_test_user_with_plan(&self.res.agent.database, &email, "professional")
                .await
                .unwrap();
        (id, tenant)
    }

    async fn connect(&self, (user, tenant): (Uuid, TenantId)) {
        let now = Utc::now();
        let repos = &self.res.common.repos;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: user,
                tenant_id: tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: SecretString::from("key-i900".to_owned()),
                refresh_token: None,
                token_type: API_KEY_TOKEN_TYPE.to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some("i900".to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(user, tenant, INTERVALS_ICU, &ConnectionType::Manual, None)
            .await
            .unwrap();
    }

    async fn call<T: McpTool<dyn ToolRuntime>>(
        &self,
        tool: &T,
        (user, tenant): (Uuid, TenantId),
        args: Value,
    ) -> (Value, bool) {
        let ctx = ToolContext::new()
            .with_user(user.to_string())
            .with_tenant(tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response = tool.execute(&self.runtime, &ctx, args).await;
        (
            response.structured_content.expect("structured content"),
            response.is_error,
        )
    }

    async fn stored(&self, (user, tenant): (Uuid, TenantId)) -> Option<UserPhysiologicalProfile> {
        self.res
            .common
            .repos
            .user_physiological_profile
            .get_user_physiological_profile(tenant, user)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn the_sex_column_round_trips_and_reads_absent_on_an_old_row() {
    let w = World::new(None).await;
    let repo: &dyn UserPhysiologicalProfileRepository =
        w.res.common.repos.user_physiological_profile.as_ref();
    let (user, tenant) = w.person().await;
    let mut profile = UserPhysiologicalProfile::new(user, SportType::Run);
    repo.upsert_user_physiological_profile(tenant, user, &profile)
        .await
        .unwrap();
    assert_eq!(w.stored((user, tenant)).await.unwrap().sex, None);

    profile.sex = Some(PhysiologicalSex::Female);
    repo.upsert_user_physiological_profile(tenant, user, &profile)
        .await
        .unwrap();
    assert_eq!(
        w.stored((user, tenant)).await.unwrap().sex,
        Some(PhysiologicalSex::Female)
    );
}

#[tokio::test]
async fn set_physiology_saves_the_stated_sex_and_the_dossier_shows_it() {
    let w = World::new(None).await;
    let who = w.person().await;

    // A save without sex leaves it absent.
    let (_, refused) = w
        .call(&SetPhysiologyTool, who, json!({ "weight": 61.0 }))
        .await;
    assert!(!refused);
    assert_eq!(w.stored(who).await.unwrap().sex, None);
    let (shown, _) = w.call(&ExportDossierTool, who, json!({})).await;
    assert!(shown["physiology"].get("sex").is_none(), "{shown:#}");

    let (body, refused) = w
        .call(&SetPhysiologyTool, who, json!({ "sex": "female" }))
        .await;
    assert!(!refused, "{body:#}");
    assert_eq!(body["profile"]["sex"], json!("female"), "{body:#}");
    assert_eq!(
        w.stored(who).await.unwrap().sex,
        Some(PhysiologicalSex::Female)
    );
    let (shown, _) = w.call(&ExportDossierTool, who, json!({})).await;
    assert_eq!(shown["physiology"]["sex"], json!("female"), "{shown:#}");

    // Only the two values the equations take are accepted.
    let (refusal, refused) = w
        .call(&SetPhysiologyTool, who, json!({ "sex": "other" }))
        .await;
    assert!(refused, "{refusal:#}");
    assert_eq!(
        w.stored(who).await.unwrap().sex,
        Some(PhysiologicalSex::Female)
    );
}

#[tokio::test]
async fn reading_the_athlete_never_writes_their_profile() {
    // get_athlete is a read: even a provider that reports a sex leaves the
    // physiological profile as it was.
    let w = World::new(Some("M")).await;
    let who = w.person().await;
    w.connect(who).await;
    let (body, refused) = w
        .call(&GetAthleteTool, who, json!({ "provider": INTERVALS_ICU }))
        .await;
    assert!(!refused, "{body:#}");
    assert!(w.stored(who).await.is_none(), "a read creates no profile");
}

#[tokio::test]
async fn the_threshold_import_records_a_provider_reported_sex_on_an_empty_profile() {
    let w = World::new(Some("M")).await;
    let who = w.person().await;
    w.connect(who).await;

    let (body, refused) = w.call(&ImportProviderThresholdsTool, who, json!({})).await;
    assert!(!refused, "{body:#}");
    assert_eq!(body["sex_recorded"], json!(true), "{body:#}");
    assert_eq!(body["saved"], json!(true), "{body:#}");
    assert_eq!(body["profile"]["sex"], json!("male"), "{body:#}");
    assert_eq!(
        w.stored(who).await.unwrap().sex,
        Some(PhysiologicalSex::Male)
    );

    // A second import finds the sex already recorded and writes nothing.
    let (again, _) = w.call(&ImportProviderThresholdsTool, who, json!({})).await;
    assert_eq!(again["sex_recorded"], json!(false), "{again:#}");
    assert_eq!(again["saved"], json!(false), "{again:#}");
}

#[tokio::test]
async fn the_athletes_own_sex_wins_over_a_providers() {
    let w = World::new(Some("M")).await;
    let who = w.person().await;
    w.connect(who).await;
    let (_, refused) = w
        .call(&SetPhysiologyTool, who, json!({ "sex": "female" }))
        .await;
    assert!(!refused);

    let (body, refused) = w.call(&ImportProviderThresholdsTool, who, json!({})).await;
    assert!(!refused, "{body:#}");
    assert_eq!(body["sex_recorded"], json!(false), "{body:#}");
    assert_eq!(
        w.stored(who).await.unwrap().sex,
        Some(PhysiologicalSex::Female),
        "a provider never replaces the athlete's statement"
    );
}

#[tokio::test]
async fn a_provider_that_reports_no_sex_records_none() {
    let w = World::new(None).await;
    let who = w.person().await;
    w.connect(who).await;
    let (body, refused) = w.call(&ImportProviderThresholdsTool, who, json!({})).await;
    assert!(!refused, "{body:#}");
    assert_eq!(body["sex_recorded"], json!(false), "{body:#}");
    assert_eq!(body["saved"], json!(false), "{body:#}");
    assert!(
        w.stored(who).await.is_none(),
        "no profile is created for nothing"
    );
}

#[tokio::test]
async fn the_lt1_pace_cohort_follows_the_saved_sex() {
    let w = World::new(None).await;
    let who = w.person().await;
    let (_, refused) = w
        .call(
            &SetPhysiologyTool,
            who,
            json!({ "threshold_pace_sec_per_km": 240.0 }),
        )
        .await;
    assert!(!refused);
    let (dossier, _) = w.call(&ExportDossierTool, who, json!({})).await;
    assert_eq!(
        dossier["threshold_estimate"]["lt1_pace"]["cohort"],
        json!("pooled"),
        "{dossier:#}"
    );

    let (_, refused) = w
        .call(&SetPhysiologyTool, who, json!({ "sex": "female" }))
        .await;
    assert!(!refused);
    let (dossier, _) = w.call(&ExportDossierTool, who, json!({})).await;
    let expected = AerobicThresholdPace::from_threshold_pace(240.0, Some(Gender::Female)).unwrap();
    let pace = &dossier["threshold_estimate"]["lt1_pace"];
    assert_eq!(pace["cohort"], json!("women"), "{dossier:#}");
    assert_eq!(
        pace["pace_sec_per_km"].as_f64(),
        Some(expected.pace_sec_per_km)
    );
}

// ---------------------------------------------------------------------------
// The migration, on each lane
// ---------------------------------------------------------------------------

/// The migration under test.
const SEX_MIGRATION: i64 = 20_261_022_090_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// The three values a planted row's `sex` is tried with, and whether the
/// column takes each.
const CASES: [(Option<&str>, bool); 4] = [
    (None, true),
    (Some("male"), true),
    (Some("female"), true),
    (Some("other"), false),
];

#[tokio::test]
async fn the_sex_column_is_nullable_and_takes_only_male_or_female() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        migrates_on_postgres(&database.url).await;
        return;
    }
    migrates_on_sqlite(&database.url).await;
}

async fn migrates_on_sqlite(url: &str) {
    // One connection: every pooled connection to an in-memory database is
    // its own empty database. Foreign keys off: the planted rows name users
    // and tenants the test does not build.
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let mut applied = false;
    for migration in MIGRATOR.iter() {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        if migration.version == SEX_MIGRATION {
            applied = true;
            break;
        }
    }
    assert!(applied, "the sex migration is in migrations/");
    for (index, (sex, accepted)) in CASES.into_iter().enumerate() {
        let result = sqlx::query(
            "INSERT INTO user_physiological_profiles (user_id, tenant_id, sex) VALUES ($1, $2, $3)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(Uuid::new_v4().to_string())
        .bind(sex)
        .execute(&pool)
        .await;
        assert_eq!(
            result.is_ok(),
            accepted,
            "case {index}: {sex:?} -> {result:?}"
        );
    }
    let stored: Vec<Option<String>> =
        sqlx::query("SELECT sex FROM user_physiological_profiles ORDER BY sex")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|row| row.get("sex"))
            .collect();
    assert_eq!(
        stored,
        [None, Some("female".to_owned()), Some("male".to_owned())]
    );
}

#[cfg(feature = "postgresql")]
async fn migrates_on_postgres(url: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    let mut applied = false;
    for migration in PG_MIGRATOR.iter() {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        if migration.version == SEX_MIGRATION {
            applied = true;
            break;
        }
    }
    assert!(applied, "the sex migration is in migrations_pg/");
    // The planted rows name users and tenants the test does not build;
    // replica mode skips the foreign-key triggers, never the CHECK.
    sqlx::raw_sql("SET session_replication_role = replica")
        .execute(&pool)
        .await
        .unwrap();
    for (index, (sex, accepted)) in CASES.into_iter().enumerate() {
        let result = sqlx::query(
            "INSERT INTO user_physiological_profiles (user_id, tenant_id, sex) VALUES ($1, $2, $3)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(sex)
        .execute(&pool)
        .await;
        assert_eq!(
            result.is_ok(),
            accepted,
            "case {index}: {sex:?} -> {result:?}"
        );
    }
}
