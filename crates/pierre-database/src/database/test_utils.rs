// ABOUTME: The single factory every test uses to open a database, honouring DATABASE_URL
// ABOUTME: PostgreSQL callers get a private clone of a migrated template database; others get in-memory SQLite
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
use crate::backends::factory::Database;
use pierre_core::errors::{AppError, AppResult};
use std::env;
#[cfg(not(feature = "postgresql"))]
use std::future::{ready, Future};

/// Environment variable the `PostgreSQL` CI lane sets.
///
/// While it is set, the factory refuses to open `SQLite`: a lane that
/// advertises `PostgreSQL` must fail loudly rather than fall back the moment
/// `DATABASE_URL` is missing or points elsewhere.
pub const REQUIRE_POSTGRES_ENV: &str = "PIERRE_TEST_REQUIRE_POSTGRES";

/// The encryption key every test database is opened with unless the caller
/// supplies its own.
const DEFAULT_TEST_KEY: [u8; 32] = [0u8; 32];

/// Create an isolated test database instance.
///
/// Uses `DATABASE_URL` when it names a `PostgreSQL` server, and in-memory
/// `SQLite` otherwise.
///
/// This previously pinned `sqlite::memory:` unconditionally — the `postgresql`
/// feature only changed the constructor signature, not the URL. `ci-postgres`
/// therefore ran the whole database suite against `SQLite`, and no test ever
/// executed the `PostgreSQL` backend's SQL. That is how a `get_by_prefix` query
/// matching `id` instead of `key_prefix` broke API-key authentication in every
/// deployed environment while its test stayed green.
///
/// # Errors
///
/// Returns an error if database initialization fails, or if
/// [`REQUIRE_POSTGRES_ENV`] is set and `DATABASE_URL` does not name a
/// `PostgreSQL` server.
pub async fn create_test_db() -> AppResult<Database> {
    create_test_db_with_key(DEFAULT_TEST_KEY.to_vec()).await
}

/// [`create_test_db`] with a caller-supplied encryption key, for tests that
/// exercise encryption, key rotation, or a second database opened under a
/// different key.
///
/// A `SQLite` database is cloned from a serialized image of a migrated
/// database — built once per process by the first caller — rather than by
/// running every embedded migration again.
///
/// # Errors
///
/// Returns an error if database initialization fails, or if
/// [`REQUIRE_POSTGRES_ENV`] is set and `DATABASE_URL` does not name a
/// `PostgreSQL` server.
pub async fn create_test_db_with_key(encryption_key: Vec<u8>) -> AppResult<Database> {
    #[cfg(feature = "postgresql")]
    if let Some(url) = postgres_test_url() {
        return postgres::create_isolated_database(&url, encryption_key).await;
    }
    refuse_sqlite_when_postgres_required()?;
    sqlite::create_isolated_database(encryption_key).await
}

/// A connection URL for a fresh, isolated test database.
///
/// For tests that hand the URL to something that opens the database itself:
/// a server booted through `ServerConfig::from_env()`, a spawned
/// `pierre-cli`, a raw introspection pool.
///
/// On `PostgreSQL` the database is cloned from the migrated template and kept
/// alive for as long as the returned handle lives; drop the handle when the
/// test is done and the next caller reclaims it. On `SQLite` the URL is the
/// shared in-memory form, which every opener turns into its own empty
/// database.
///
/// # Errors
///
/// Returns an error if the database cannot be created, or if
/// [`REQUIRE_POSTGRES_ENV`] is set and `DATABASE_URL` does not name a
/// `PostgreSQL` server.
#[cfg(feature = "postgresql")]
pub async fn create_test_db_url() -> AppResult<TestDatabaseUrl> {
    if let Some(url) = postgres_test_url() {
        return postgres::create_isolated_database_url(&url).await;
    }
    refuse_sqlite_when_postgres_required()?;
    Ok(TestDatabaseUrl {
        url: "sqlite::memory:".to_owned(),
        keeper: None,
    })
}

/// A connection URL for a fresh, isolated test database.
///
/// Without the `postgresql` feature the only backend is in-memory `SQLite`,
/// whose URL needs no server round-trip; the result is still a future so
/// callers await it the same way on both builds.
///
/// # Errors
///
/// The future resolves to an error if [`REQUIRE_POSTGRES_ENV`] is set, since
/// this build cannot open `PostgreSQL` at all.
#[cfg(not(feature = "postgresql"))]
pub fn create_test_db_url() -> impl Future<Output = AppResult<TestDatabaseUrl>> {
    ready(
        refuse_sqlite_when_postgres_required().map(|()| TestDatabaseUrl {
            url: "sqlite::memory:".to_owned(),
        }),
    )
}

/// The URL of an isolated test database, alive for as long as this value is.
#[derive(Debug)]
pub struct TestDatabaseUrl {
    /// The connection URL. Opening it yields the isolated database.
    pub url: String,
    /// One open session on the `PostgreSQL` database, so garbage collection
    /// sees it as in use until the handle is dropped.
    #[cfg(feature = "postgresql")]
    keeper: Option<sqlx::PgConnection>,
}

impl TestDatabaseUrl {
    /// Whether the database behind [`url`](Self::url) is reserved for this
    /// handle: a `PostgreSQL` database is held open until the handle drops so
    /// nothing reclaims it under the test; a `SQLite` URL needs no reservation
    /// because every opener gets its own in-memory database.
    #[must_use]
    pub const fn is_reserved(&self) -> bool {
        #[cfg(feature = "postgresql")]
        {
            self.keeper.is_some()
        }
        #[cfg(not(feature = "postgresql"))]
        {
            false
        }
    }
}

/// Create an in-memory `SQLite` test database, ignoring `DATABASE_URL`.
///
/// For the tests that exercise the `SQLite` backend itself — a raw
/// `Database::sqlite_pool()`, a `SqliteDatabase` method, or a comparison of
/// the two dialects' schemas. Those tests depend on the backend by
/// construction, so they say so explicitly rather than relying on
/// [`create_test_db`] happening to return `SQLite`. They live in files named
/// `*_sqlite_test.rs`, which the `PostgreSQL` lane does not select.
///
/// # Errors
///
/// Returns an error if database initialization fails.
pub async fn create_sqlite_test_db() -> AppResult<Database> {
    sqlite::create_isolated_database(DEFAULT_TEST_KEY.to_vec()).await
}

/// `DATABASE_URL` when it points at a `PostgreSQL` server, else `None`.
///
/// A `SQLite` `DATABASE_URL` (what local runs and the non-postgres CI job set)
/// falls through to the in-memory default, so this never redirects a test at a
/// developer's on-disk database.
fn postgres_test_url() -> Option<String> {
    let url = env::var("DATABASE_URL").ok()?;
    (url.starts_with("postgres://") || url.starts_with("postgresql://")).then_some(url)
}

/// Fail instead of opening `SQLite` when the caller's lane requires `PostgreSQL`.
fn refuse_sqlite_when_postgres_required() -> AppResult<()> {
    if env::var_os(REQUIRE_POSTGRES_ENV).is_none_or(|value| value.is_empty()) {
        return Ok(());
    }
    let reason = if cfg!(feature = "postgresql") {
        match postgres_test_url() {
            Some(_) => return Ok(()),
            None => "DATABASE_URL does not name a PostgreSQL server",
        }
    } else {
        "this binary was built without the `postgresql` feature"
    };
    Err(AppError::config(format!(
        "{REQUIRE_POSTGRES_ENV} is set but {reason}; refusing to open SQLite in a lane that \
         advertises PostgreSQL"
    )))
}

mod sqlite {
    //! Per-test in-memory `SQLite` databases cloned from a serialized image.
    //!
    //! Running the embedded migrations for every factory call cost seconds
    //! per test at test-profile optimization levels. `SQLite` exposes
    //! `sqlite3_serialize`/`sqlite3_deserialize`, so the first call in the
    //! process migrates one throwaway database and serializes it; every
    //! later call deserializes those bytes into a fresh in-memory database,
    //! re-creating the finished schema in milliseconds. The image can never
    //! go stale within a process because the embedded migration set is fixed
    //! per binary — the `PostgreSQL` template's fingerprint solves the same
    //! problem across processes, which `SQLite`'s process-local bytes never
    //! face.

    use super::DEFAULT_TEST_KEY;
    use crate::backends::factory::Database;
    use crate::backends::shared;
    use crate::database::Database as SqliteDatabase;
    use pierre_core::errors::{AppError, AppResult};
    use sqlx::sqlite::{SqliteOwnedBuf, SqlitePoolOptions};
    use std::collections::HashMap;
    use tokio::sync::OnceCell;

    /// Serialized form of a fully migrated, otherwise empty database, built
    /// by the first factory call in the process and cloned by every later
    /// one. `tokio`'s cell rather than `std`'s because the build is async;
    /// racing first callers block on it and it runs exactly once.
    static MIGRATED_IMAGE: OnceCell<Vec<u8>> = OnceCell::const_new();

    /// Open a fresh in-memory database carrying the migrated schema, wrapped
    /// under `encryption_key`.
    pub(super) async fn create_isolated_database(encryption_key: Vec<u8>) -> AppResult<Database> {
        let image = MIGRATED_IMAGE.get_or_try_init(build_image).await?;

        // Deserializing swaps the database behind ONE connection, and an
        // in-memory database is invisible to every other connection, so the
        // pool is pinned to that single connection and never recycles it —
        // a recycled connection would silently swap the test's data for an
        // empty database. Concurrent statements queue on the one connection,
        // which is how SQLite orders concurrent writers anyway.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect("sqlite::memory:")
            .await
            .map_err(|e| {
                AppError::database(format!("Test DB: cannot open in-memory SQLite: {e}"))
            })?;
        let staged = SqliteOwnedBuf::try_from(image.as_slice()).map_err(|e| {
            AppError::database(format!("Test DB: cannot stage the schema image: {e}"))
        })?;
        let mut conn = pool.acquire().await.map_err(|e| {
            AppError::database(format!("Test DB: cannot reach the in-memory SQLite: {e}"))
        })?;
        conn.deserialize(None, staged, false).await.map_err(|e| {
            AppError::database(format!("Test DB: cannot load the schema image: {e}"))
        })?;
        drop(conn);
        Ok(Database::SQLite(wrap_migrated_pool(pool, encryption_key)))
    }

    /// Wrap a pool whose database already carries the fully migrated schema
    /// — the clone-from-image path skips the migration run, so it fills the
    /// backend's fields directly (a sibling module may touch them; production
    /// construction stays in `Database::new`).
    fn wrap_migrated_pool(
        pool: sqlx::Pool<sqlx::Sqlite>,
        encryption_key: Vec<u8>,
    ) -> SqliteDatabase {
        SqliteDatabase {
            pool,
            blind_index_key: encryption_key.clone(),
            active_dek_version: shared::encryption::LEGACY_DEK_VERSION,
            prior_dek_versions: HashMap::new(),
            encryption_key,
        }
    }

    /// Migrate one throwaway in-memory database and serialize its `main`
    /// schema. Migrations write no encrypted data, so the throwaway's key is
    /// irrelevant and one image serves every caller-supplied key.
    async fn build_image() -> AppResult<Vec<u8>> {
        let source = SqliteDatabase::new("sqlite::memory:", DEFAULT_TEST_KEY.to_vec()).await?;
        let mut conn = source.pool().acquire().await.map_err(|e| {
            AppError::database(format!("Test DB: cannot reach the image source: {e}"))
        })?;
        let image = conn.serialize(None).await.map_err(|e| {
            AppError::database(format!(
                "Test DB: cannot serialize the migrated schema: {e}"
            ))
        })?;
        Ok(image.as_ref().to_vec())
    }
}

#[cfg(feature = "postgresql")]
mod postgres {
    //! Per-test `PostgreSQL` databases cloned from a migrated template.
    //!
    //! Running the embedded migrations for every test cost seconds per test
    //! on the CI lane; `CREATE DATABASE … TEMPLATE …` copies the finished
    //! template's files in well under a second. The template is built once
    //! per server per migration set — its name carries the set's fingerprint,
    //! so a binary compiled against different migrations builds its own — and
    //! that build is the only step that takes a lock: concurrent tests clone
    //! the template in parallel, which is what keeps four test threads from
    //! queueing behind one another's setup.
    //!
    //! Finished databases are reclaimed lazily: each creation first drops
    //! every test database that no session is connected to and that is at
    //! least [`RECLAIM_AFTER`] old. The age gate is what makes reclamation
    //! safe without a lock — a database is connected to within milliseconds
    //! of its creation, so one that is both unconnected and older than the
    //! gate belongs to a test that has finished.

    use super::TestDatabaseUrl;
    use crate::backends::factory::Database;
    use crate::backends::postgres::migrations_fingerprint;
    use pierre_core::config::database::PostgresPoolConfig;
    use pierre_core::errors::{AppError, AppResult};
    use sqlx::{Connection, Executor, PgConnection, Row};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

    /// Advisory lock held while the template is being built. The value is
    /// arbitrary and only has to be the same for every caller.
    const TEMPLATE_BUILD_LOCK_KEY: i64 = 0x7069_6572_7265_7473;

    /// Prefix of every per-test database; the creation time in Unix
    /// milliseconds follows it, so reclamation can read a database's age from
    /// its name alone.
    const TEST_DB_PREFIX: &str = "pierre_test_";

    /// Prefix of every template; the migration fingerprint follows it.
    const TEMPLATE_PREFIX: &str = "pierre_tpl_";

    /// How old an unconnected test database must be before it is reclaimed.
    /// Far longer than the gap between a database's creation and its first
    /// session, far shorter than the runs that would otherwise let hundreds
    /// of finished databases accumulate.
    const RECLAIM_AFTER: Duration = Duration::from_secs(10);

    /// Clone a fresh database for one test and open it under `encryption_key`.
    pub(super) async fn create_isolated_database(
        base_url: &str,
        encryption_key: Vec<u8>,
    ) -> AppResult<Database> {
        let mut admin = connect_admin(base_url).await?;
        let outcome = async {
            let name = clone_template(&mut admin, base_url).await?;
            let url = with_database(base_url, &name);
            Database::new(&url, encryption_key, &PostgresPoolConfig::default()).await
        }
        .await;
        drop_connection(admin).await;
        outcome
    }

    /// Clone a fresh database for one test and return its URL, keeping one
    /// session open on it so the database stays reserved.
    pub(super) async fn create_isolated_database_url(base_url: &str) -> AppResult<TestDatabaseUrl> {
        let mut admin = connect_admin(base_url).await?;
        let outcome = async {
            let name = clone_template(&mut admin, base_url).await?;
            let url = with_database(base_url, &name);
            let keeper = PgConnection::connect(&url).await.map_err(|e| {
                AppError::database(format!("Test DB: cannot open a session on {name}: {e}"))
            })?;
            Ok(TestDatabaseUrl {
                url,
                keeper: Some(keeper),
            })
        }
        .await;
        drop_connection(admin).await;
        outcome
    }

    /// Make sure the template exists, reclaim finished databases, and clone
    /// the template. Returns the new database's name.
    async fn clone_template(admin: &mut PgConnection, base_url: &str) -> AppResult<String> {
        let template = ensure_template(admin, base_url).await?;
        reclaim_unused(admin, &template).await?;
        let name = format!(
            "{TEST_DB_PREFIX}{}_{}",
            unix_millis(),
            &Uuid::new_v4().simple().to_string()[..12]
        );
        // A concurrent run compiled against a different migration set reclaims
        // this run's template as foreign, and a template mid-rebuild briefly
        // has sessions on it — both surface here, between the existence check
        // and the clone. Rebuilding and going again is always safe, so a racy
        // failure retries instead of failing the test.
        let mut attempts = 0;
        loop {
            let outcome = admin
                .execute(format!("CREATE DATABASE {name} TEMPLATE {template}").as_str())
                .await;
            match outcome {
                Ok(_) => break,
                Err(e) => {
                    attempts += 1;
                    let text = e.to_string();
                    let racy = text.contains("does not exist")
                        || text.contains("is being accessed by other users");
                    if attempts >= 3 || !racy {
                        return Err(AppError::database(format!(
                            "Test DB: cannot clone {template} into {name}: {e}"
                        )));
                    }
                    ensure_template(admin, base_url).await?;
                }
            }
        }
        Ok(name)
    }

    /// The template for this binary's migration set, building it if this is
    /// the first caller on the server to need it.
    ///
    /// The build runs under an advisory lock and re-checks after taking it,
    /// so concurrent first callers — four threads in one binary, or several
    /// binaries sharing a server — build it exactly once.
    async fn ensure_template(admin: &mut PgConnection, base_url: &str) -> AppResult<String> {
        let template = format!("{TEMPLATE_PREFIX}{}", migrations_fingerprint());
        if database_exists(admin, &template).await? {
            return Ok(template);
        }
        lock(admin).await?;
        let outcome = async {
            if database_exists(admin, &template).await? {
                return Ok(());
            }
            build_template(admin, base_url, &template).await
        }
        .await;
        unlock(admin).await?;
        outcome.map(|()| template)
    }

    /// Create the template database and apply every embedded migration to it.
    ///
    /// The template is built under a `_building` name and renamed only once
    /// migrations have completed and every session on it is closed, so a
    /// crash mid-build can never leave a half-migrated database wearing the
    /// finished template's name.
    async fn build_template(
        admin: &mut PgConnection,
        base_url: &str,
        template: &str,
    ) -> AppResult<()> {
        let building = format!("{template}_building");
        admin
            .execute(format!("DROP DATABASE IF EXISTS {building}").as_str())
            .await
            .map_err(|e| {
                AppError::database(format!("Test DB: cannot discard stale {building}: {e}"))
            })?;
        admin
            .execute(format!("CREATE DATABASE {building}").as_str())
            .await
            .map_err(|e| AppError::database(format!("Test DB: cannot create {building}: {e}")))?;

        let migrated = Database::new(
            &with_database(base_url, &building),
            vec![0u8; 32],
            &PostgresPoolConfig::default(),
        )
        .await?;
        if let Some(pool) = migrated.postgres_pool() {
            pool.close().await;
        }
        drop(migrated);

        admin
            .execute(format!("ALTER DATABASE {building} RENAME TO {template}").as_str())
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Test DB: cannot publish {building} as {template}: {e}"
                ))
            })?;
        Ok(())
    }

    /// Drop every test database that no session is connected to and that is
    /// older than [`RECLAIM_AFTER`], and every template for another migration
    /// set that no session is connected to.
    ///
    /// A database whose drop loses the race to a new session, or to another
    /// caller reclaiming the same name, is simply skipped: it is either in
    /// use after all or already gone.
    async fn reclaim_unused(admin: &mut PgConnection, current_template: &str) -> AppResult<()> {
        let rows = sqlx::query(
            "SELECT d.datname FROM pg_database d \
             WHERE (d.datname LIKE $1 OR (d.datname LIKE $2 AND d.datname <> $3)) \
               AND NOT EXISTS (SELECT 1 FROM pg_stat_activity a WHERE a.datname = d.datname)",
        )
        .bind(format!("{TEST_DB_PREFIX}%"))
        .bind(format!("{TEMPLATE_PREFIX}%"))
        .bind(current_template)
        .fetch_all(&mut *admin)
        .await
        .map_err(|e| {
            AppError::database(format!("Test DB: cannot list reclaimable databases: {e}"))
        })?;
        let now = unix_millis();
        for row in rows {
            let name: String = row.try_get("datname").map_err(|e| {
                AppError::database(format!("Test DB: unreadable datname while reclaiming: {e}"))
            })?;
            if let Some(created) = creation_millis(&name) {
                if now.saturating_sub(created) < RECLAIM_AFTER.as_millis() {
                    continue;
                }
            }
            // A `_building` database is a template mid-construction under the
            // build lock; dropping it out from under its builder is the one
            // reclamation that breaks a live caller. Its own builder discards
            // a stale one before reusing the name.
            if name.ends_with("_building") {
                continue;
            }
            let _ = admin
                .execute(format!("DROP DATABASE IF EXISTS {name}").as_str())
                .await;
        }
        Ok(())
    }

    /// The creation time a test database's name carries, if it is one.
    fn creation_millis(name: &str) -> Option<u128> {
        name.strip_prefix(TEST_DB_PREFIX)?
            .split('_')
            .next()?
            .parse()
            .ok()
    }

    fn unix_millis() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis())
    }

    async fn database_exists(admin: &mut PgConnection, name: &str) -> AppResult<bool> {
        let row = sqlx::query("SELECT 1 FROM pg_database WHERE datname = $1")
            .bind(name)
            .fetch_optional(&mut *admin)
            .await
            .map_err(|e| {
                AppError::database(format!("Test DB: cannot look up database {name}: {e}"))
            })?;
        Ok(row.is_some())
    }

    async fn connect_admin(base_url: &str) -> AppResult<PgConnection> {
        PgConnection::connect(base_url)
            .await
            .map_err(|e| AppError::database(format!("Test DB: cannot reach PostgreSQL: {e}")))
    }

    async fn lock(admin: &mut PgConnection) -> AppResult<()> {
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(TEMPLATE_BUILD_LOCK_KEY)
            .execute(&mut *admin)
            .await
            .map(|_| ())
            .map_err(|e| {
                AppError::database(format!("Test DB: cannot take the template build lock: {e}"))
            })
    }

    async fn unlock(admin: &mut PgConnection) -> AppResult<()> {
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(TEMPLATE_BUILD_LOCK_KEY)
            .execute(&mut *admin)
            .await
            .map(|_| ())
            .map_err(|e| {
                AppError::database(format!(
                    "Test DB: cannot release the template build lock: {e}"
                ))
            })
    }

    /// Close the admin session so it never counts as a user of any database.
    async fn drop_connection(admin: PgConnection) {
        // A session that fails to close cleanly still ends when it is dropped;
        // the lock it might hold is session-scoped and dies with it either way.
        let _ = admin.close().await;
    }

    /// `base_url` with its database path replaced by `name`, query string kept.
    ///
    /// Works on the `scheme://user:password@host:port/database?params` shape
    /// every `DATABASE_URL` in this project takes; a password containing a
    /// raw `/` must be percent-encoded, as `PostgreSQL` itself requires.
    fn with_database(base_url: &str, name: &str) -> String {
        let (head, query) = match base_url.split_once('?') {
            Some((head, query)) => (head, Some(query)),
            None => (base_url, None),
        };
        let authority_start = head.find("://").map_or(0, |i| i + 3);
        let path_start = head[authority_start..]
            .find('/')
            .map_or(head.len(), |i| authority_start + i);
        let mut url = format!("{}/{name}", &head[..path_start]);
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }
        url
    }
}
