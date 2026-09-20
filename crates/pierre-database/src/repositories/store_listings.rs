// ABOUTME: Repository trait for the agent marketplace: the review workflow, catalogue browsing, installs and the @handle
// ABOUTME: Every statement written once as a shared const, one generic listing parser, one macro emitting each backend's impl
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{
    Agent, AgentCategory, AgentHandle, AgentWithListing, PublishStatus, StoreAdminStats,
    StoreListing,
};
use pierre_core::models::TenantId;
use pierre_core::pagination::{CursorPage, StoreSortOrder};
use sqlx::{ColumnIndex, Decode, Row, Type};
use uuid::Uuid;

/// Store listings for the agent marketplace (cross-tenant browsing, install/uninstall)
#[async_trait]
pub trait StoreListingsRepository: Send + Sync {
    /// Submit an agent for Store review (creates listing if needed)
    async fn submit_for_review(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing>;
    /// Get a store listing by agent ID
    async fn get_listing(&self, agent_id: &str) -> AppResult<Option<StoreListing>>;
    /// Approve an agent and publish to the Store
    async fn approve_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
    ) -> AppResult<AgentWithListing>;
    /// Reject an agent with a reason
    async fn reject_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
        reason: &str,
    ) -> AppResult<AgentWithListing>;
    /// Unpublish an agent (revert from published to draft)
    async fn unpublish_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<AgentWithListing>;
    /// Get agents pending admin review
    async fn get_pending_review_agents(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<AgentWithListing>>;
    /// Get agents that have been rejected
    async fn get_rejected_agents(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<AgentWithListing>>;
    /// Get store admin statistics
    async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats>;
    /// Get author email for an agent
    async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>>;
    /// Get published agents for the Store (cross-tenant)
    async fn get_published_agents(
        &self,
        category: Option<AgentCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<AgentWithListing>>;
    /// Get published agents with cursor-based pagination
    async fn get_published_agents_cursor(
        &self,
        category: Option<AgentCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<AgentWithListing>>;
    /// Search published agents by title/description/tags, in `locale`.
    ///
    /// The canonical English row and the `agent_translations` overlay for
    /// `locale` are both matched, so an athlete searching the words the Store
    /// showed her — a chip reading `methode-norvegienne`, say — reaches the
    /// agent whose canonical tag is `norwegian-method`. Matching only the
    /// canonical row made every localized label unsearchable; matching only
    /// the overlay would lose the agents that have no translation.
    async fn search_published_agents(
        &self,
        query: &str,
        limit: Option<u32>,
        locale: &str,
    ) -> AppResult<Vec<AgentWithListing>>;
    /// Get a single published agent by ID (cross-tenant)
    async fn get_published_agent(&self, agent_id: &str) -> AppResult<Option<AgentWithListing>>;
    /// Resolve a published catalogue agent by its `@handle` (cross-tenant).
    ///
    /// The origin agent only — the row that owns the handle, never an
    /// athlete's installed copy (which carries the handle as a reference) —
    /// and only while its listing is published, so an agent that left the
    /// Store is no longer installable by name.
    async fn find_published_by_handle(
        &self,
        handle: &AgentHandle,
    ) -> AppResult<Option<AgentWithListing>>;
    /// Get category counts for published agents
    async fn get_category_counts(&self) -> AppResult<HashMap<AgentCategory, i64>>;
    /// Increment install count for an agent's store listing
    async fn increment_install_count(&self, agent_id: &str) -> AppResult<()>;
    /// Decrement install count for an agent's store listing
    async fn decrement_install_count(&self, agent_id: &str) -> AppResult<()>;
    /// Install an agent from the Store (creates user's copy)
    async fn install_from_store(
        &self,
        source_agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Agent>;
    /// Uninstall an agent (deletes user's copy, returns source agent ID)
    async fn uninstall_agent(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String>;
    /// Get user's installed agents from the Store
    async fn get_installed_agents(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Agent>>;
    /// Create or ensure a store listing exists for an agent
    async fn ensure_listing(&self, agent_id: &str, tenant_id: TenantId) -> AppResult<StoreListing>;
    /// Give an agent its catalogue `@handle` if it owns none yet, and return it.
    ///
    /// The same assignment Store approval performs, exposed for an agent
    /// created outside the Store (`/agent create`) so `@handle` and
    /// `/agent add @handle` reach it from the moment it exists. An origin
    /// agent already carrying a handle keeps it; otherwise the first free
    /// candidate derived from the title is taken at catalogue scope.
    async fn assign_catalogue_handle(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<String>;
}

// ============================================================================
// Statements
// ============================================================================
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres, so one statement serves both backends and cannot drift between
// them. Three id columns differ in type between the schemas and bind through
// the backend's uuid codec (see [`super::uuid_columns`]): `users.id`,
// `agents.user_id` and `agent_assignments.user_id`/`assigned_by`, `uuid` on
// Postgres and hyphenated `TEXT` on `SQLite`. `agents.tenant_id` is `uuid` on
// Postgres and `TEXT` on `SQLite` too, and [`TenantId`] carries its own codec
// for exactly that split, so it binds as itself. `store_listings.tenant_id`,
// `store_listings.id`, `store_listings.agent_id` and `agents.id` are `TEXT`
// on both schemas and bind as their strings. Timestamps bind as
// `DateTime<Utc>` on both: sqlx-sqlite writes the same RFC 3339 text
// (`to_rfc3339_opts(AutoSi, false)`, which is what `to_rfc3339()` renders)
// the column already holds, so the cursor pages' text comparison stays exact
// on `SQLite`; Postgres writes its native `TIMESTAMPTZ`.

/// The nineteen `agents` columns every read of an agent returns, in the
/// order both backends' agent row parsers read them and the install `INSERT`
/// names them.
macro_rules! agent_columns {
    () => {
        "id, user_id, tenant_id, title, description, system_prompt, \
         category, tags, sample_prompts, token_count, \
         created_at, updated_at, is_system, visibility, prerequisites, \
         forked_from, slug, max_tool_iterations, temperature"
    };
}

/// The same nineteen columns qualified by the `c` alias of a join.
macro_rules! agent_columns_aliased {
    () => {
        "c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt, \
         c.category, c.tags, c.sample_prompts, c.token_count, \
         c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites, \
         c.forked_from, c.slug, c.max_tool_iterations, c.temperature"
    };
}
pub(crate) use agent_columns_aliased;

/// The listing columns of a join, qualified by the `sl` alias; `id`,
/// `created_at` and `updated_at` are renamed so they do not collide with the
/// agent's own.
macro_rules! listing_columns_aliased {
    () => {
        "sl.id as sl_id, sl.publish_status, sl.published_at, \
         sl.review_submitted_at, sl.review_decision_at, sl.review_decision_by, \
         sl.rejection_reason, sl.install_count, sl.icon_url, sl.author_id, \
         sl.created_at as sl_created_at, sl.updated_at as sl_updated_at"
    };
}
pub(crate) use listing_columns_aliased;

/// An agent joined to its listing, for the statements built at runtime.
macro_rules! agent_with_listing_select {
    () => {
        concat!(
            "SELECT ",
            agent_columns_aliased!(),
            ", ",
            listing_columns_aliased!(),
            " FROM agents c JOIN store_listings sl ON c.id = sl.agent_id "
        )
    };
}
pub(crate) use agent_with_listing_select;

/// The agent the caller owns, if any — `agents.user_id` and `agents.tenant_id`
/// bind through the codecs above.
pub(crate) const OWNED_AGENT_SQL: &str =
    "SELECT id, tenant_id FROM agents WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// The listing an agent already has, with its status.
pub(crate) const EXISTING_LISTING_SQL: &str =
    "SELECT id, publish_status FROM store_listings WHERE agent_id = $1";

/// Move a draft listing into review; `$2` is the call time, bound to both
/// timestamps.
pub(crate) const SUBMIT_LISTING_SQL: &str = "UPDATE store_listings SET \
     publish_status = $1, review_submitted_at = $2, updated_at = $2 \
     WHERE id = $3";

/// Reflect a listing change on the agent's own `updated_at`.
pub(crate) const TOUCH_AGENT_SQL: &str = "UPDATE agents SET updated_at = $1 WHERE id = $2";

/// Create a listing straight into review; `$5` is the call time, bound to
/// the submission and both row timestamps.
pub(crate) const INSERT_PENDING_LISTING_SQL: &str = "INSERT INTO store_listings ( \
     id, agent_id, tenant_id, publish_status, review_submitted_at, \
     install_count, created_at, updated_at \
     ) VALUES ($1, $2, $3, $4, $5, 0, $5, $5)";

/// Create a draft listing; `$5` is the call time, bound to both timestamps.
pub(crate) const INSERT_DRAFT_LISTING_SQL: &str = "INSERT INTO store_listings ( \
     id, agent_id, tenant_id, publish_status, install_count, created_at, updated_at \
     ) VALUES ($1, $2, $3, $4, 0, $5, $5)";

/// One agent's listing, every column.
pub(crate) const GET_LISTING_SQL: &str = "SELECT id, agent_id, tenant_id, publish_status, \
     published_at, review_submitted_at, review_decision_at, review_decision_by, \
     rejection_reason, install_count, icon_url, author_id, created_at, updated_at \
     FROM store_listings WHERE agent_id = $1";

/// Publish a listing under review; `$2` is the decision time, bound to
/// `published_at`, `review_decision_at` and `updated_at`.
pub(crate) const APPROVE_LISTING_SQL: &str = "UPDATE store_listings SET \
     publish_status = $1, published_at = $2, review_decision_at = $2, \
     review_decision_by = $3, rejection_reason = NULL, updated_at = $2 \
     WHERE agent_id = $4 AND tenant_id = $5 AND publish_status = 'pending_review'";

/// Reject a listing under review with a reason; `$2` is the decision time.
pub(crate) const REJECT_LISTING_SQL: &str = "UPDATE store_listings SET \
     publish_status = $1, review_decision_at = $2, review_decision_by = $3, \
     rejection_reason = $4, updated_at = $2 \
     WHERE agent_id = $5 AND tenant_id = $6 AND publish_status = 'pending_review'";

/// Take a published listing back to draft.
pub(crate) const UNPUBLISH_LISTING_SQL: &str = "UPDATE store_listings SET \
     publish_status = $1, published_at = NULL, updated_at = $2 \
     WHERE agent_id = $3 AND tenant_id = $4 AND publish_status = 'published'";

/// A tenant's listings under review, oldest submission first.
pub(crate) const PENDING_REVIEW_SQL: &str = concat!(
    agent_with_listing_select!(),
    "WHERE sl.tenant_id = $1 AND sl.publish_status = 'pending_review' \
     ORDER BY sl.review_submitted_at ASC LIMIT $2 OFFSET $3"
);

/// A tenant's rejected listings, newest decision first.
pub(crate) const REJECTED_SQL: &str = concat!(
    agent_with_listing_select!(),
    "WHERE sl.tenant_id = $1 AND sl.publish_status = 'rejected' \
     ORDER BY sl.review_decision_at DESC LIMIT $2 OFFSET $3"
);

/// A tenant's review counts and the installs of its published agents.
pub(crate) const STORE_ADMIN_STATS_SQL: &str = "SELECT \
     COUNT(CASE WHEN publish_status = 'pending_review' THEN 1 END) as pending_count, \
     COUNT(CASE WHEN publish_status = 'published' THEN 1 END) as published_count, \
     COUNT(CASE WHEN publish_status = 'rejected' THEN 1 END) as rejected_count, \
     COALESCE(SUM(CASE WHEN publish_status = 'published' THEN install_count ELSE 0 END), 0) as total_installs \
     FROM store_listings WHERE tenant_id = $1";

/// The email behind an author id; `users.id` binds through the codec.
pub(crate) const AUTHOR_EMAIL_SQL: &str = "SELECT email FROM users WHERE id = $1";

/// One published agent, whichever tenant published it.
pub(crate) const PUBLISHED_AGENT_SQL: &str = concat!(
    agent_with_listing_select!(),
    "WHERE c.id = $1 AND sl.publish_status = 'published'"
);

/// The published origin agent answering to a handle. `forked_from IS NULL`
/// keeps installed copies out: they carry the origin's handle as a reference
/// and never own a listing of their own.
pub(crate) const PUBLISHED_BY_HANDLE_SQL: &str = concat!(
    agent_with_listing_select!(),
    "WHERE c.slug = $1 AND c.forked_from IS NULL AND sl.publish_status = 'published' LIMIT 1"
);

/// An agent with its listing, within the listing's tenant.
pub(crate) const AGENT_WITH_LISTING_SQL: &str = concat!(
    agent_with_listing_select!(),
    "WHERE c.id = $1 AND sl.tenant_id = $2"
);

/// How many published agents each category holds.
pub(crate) const CATEGORY_COUNTS_SQL: &str = "SELECT c.category, COUNT(*) as count \
     FROM agents c JOIN store_listings sl ON c.id = sl.agent_id \
     WHERE sl.publish_status = 'published' GROUP BY c.category";

/// One more install of a published agent.
pub(crate) const INCREMENT_INSTALLS_SQL: &str = "UPDATE store_listings \
     SET install_count = install_count + 1, updated_at = $1 \
     WHERE agent_id = $2 AND publish_status = 'published'";

/// One install fewer, never below zero. The `CASE` is the clamp both engines
/// accept: `SQLite` has no `GREATEST`, and on Postgres `MAX` is an aggregate.
pub(crate) const DECREMENT_INSTALLS_SQL: &str = "UPDATE store_listings \
     SET install_count = CASE WHEN install_count > 0 THEN install_count - 1 ELSE 0 END, \
     updated_at = $1 \
     WHERE agent_id = $2";

/// The caller's existing copy of a Store agent, if any.
pub(crate) const EXISTING_INSTALL_SQL: &str =
    "SELECT id FROM agents WHERE user_id = $1 AND tenant_id = $2 AND forked_from = $3";

/// Create the caller's personal copy of a Store agent. `$11` is the call
/// time, bound to both timestamps; `FALSE` is the boolean spelling both
/// engines accept for `is_system`.
pub(crate) const INSTALL_AGENT_SQL: &str = "INSERT INTO agents ( \
     id, user_id, tenant_id, title, description, system_prompt, category, tags, \
     sample_prompts, token_count, \
     created_at, updated_at, is_system, visibility, prerequisites, forked_from, slug \
     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, FALSE, $12, $13, $14, $15)";

/// Self-assign the installed copy. `ON CONFLICT DO NOTHING` keeps an
/// existing `(agent_id, user_id)` row on both engines.
pub(crate) const SELF_ASSIGN_SQL: &str = "INSERT INTO agent_assignments ( \
     id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at \
     ) VALUES ($1, $2, $3, $3, $4, FALSE, 0, NULL) ON CONFLICT DO NOTHING";

/// The copy just installed, owner-scoped.
pub(crate) const INSTALLED_AGENT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(),
    " FROM agents WHERE id = $1 AND user_id = $2 AND tenant_id = $3"
);

/// The caller's agent with its origin, for an uninstall.
pub(crate) const OWNED_AGENT_ORIGIN_SQL: &str =
    "SELECT id, forked_from FROM agents WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Delete the caller's copy.
pub(crate) const DELETE_OWNED_AGENT_SQL: &str =
    "DELETE FROM agents WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Every agent the caller installed from the Store, newest first.
pub(crate) const INSTALLED_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(),
    " FROM agents WHERE user_id = $1 AND tenant_id = $2 AND forked_from IS NOT NULL \
     ORDER BY created_at DESC"
);

/// The agent a handle is being assigned to: its title, the handle it owns,
/// and whether it is a copy.
pub(crate) const HANDLE_OWNER_SQL: &str =
    "SELECT title, slug, forked_from FROM agents WHERE id = $1 AND tenant_id = $2";

/// Whether another origin or published agent already answers to a handle.
pub(crate) const HANDLE_TAKEN_SQL: &str = "SELECT 1 FROM agents WHERE slug = $1 AND id <> $2 \
     AND (forked_from IS NULL \
     OR id IN (SELECT agent_id FROM store_listings WHERE publish_status = 'published')) \
     LIMIT 1";

/// Give an agent its handle.
pub(crate) const ASSIGN_HANDLE_SQL: &str = "UPDATE agents SET slug = $1 WHERE id = $2";

/// A page of published agents whose title, description or tags — or the
/// `locale` overlay's — contain the query, most installed first. `$like` is
/// the backend's case-folding match operator.
///
/// The overlay join is what makes a localized tag findable: the chips the
/// athlete reads come from `agent_translations.tags`, while the canonical
/// slug the agent was published under stays on `agents`. `(agent_id, locale)`
/// is the overlay's primary key, so the join adds at most one row per agent.
macro_rules! search_published_sql {
    ($like:literal) => {
        concat!(
            agent_with_listing_select!(),
            "LEFT JOIN agent_translations ct ON ct.agent_id = c.id AND ct.locale = $2 \
             WHERE sl.publish_status = 'published' AND (c.title ",
            $like,
            " $1 OR c.description ",
            $like,
            " $1 OR c.tags ",
            $like,
            " $1 OR ct.title ",
            $like,
            " $1 OR ct.description ",
            $like,
            " $1 OR ct.tags ",
            $like,
            " $1) ORDER BY sl.install_count DESC, sl.published_at DESC LIMIT $3"
        )
    };
}
pub(crate) use search_published_sql;

/// Newest first, then id, for the cursor pages.
pub(crate) const NEWEST_ORDER: &str = "sl.published_at DESC, c.id DESC";
/// The rows after a newest-sort cursor: `$1` its `published_at`, `$2` its id.
pub(crate) const NEWEST_AFTER: &str =
    "AND (sl.published_at < $1 OR (sl.published_at = $1 AND c.id < $2))";
/// Most installed first, then newest, then id.
pub(crate) const POPULAR_ORDER: &str = "sl.install_count DESC, sl.published_at DESC, c.id DESC";
/// The rows after a popular-sort cursor: `$1` its install count, `$2` its
/// `published_at`, `$3` its id.
pub(crate) const POPULAR_AFTER: &str = "AND (sl.install_count < $1 \
     OR (sl.install_count = $1 AND sl.published_at < $2) \
     OR (sl.install_count = $1 AND sl.published_at = $2 AND c.id < $3))";
/// Title order, then id.
pub(crate) const TITLE_ORDER: &str = "c.title ASC, c.id ASC";
/// The rows after a title-sort cursor: `$1` its title, `$2` its id.
pub(crate) const TITLE_AFTER: &str = "AND (c.title > $1 OR (c.title = $1 AND c.id > $2))";

/// The `AND c.category = '…'` filter of a browse, or nothing. The value is
/// the enum's own spelling, never caller text.
pub(crate) fn category_filter(category: Option<AgentCategory>) -> String {
    category.map_or_else(String::new, |cat| {
        format!("AND c.category = '{}'", cat.as_str())
    })
}

/// The offset page of published agents a browse asks for, `$1` the limit
/// and `$2` the offset.
pub(crate) fn published_agents_sql(category_filter: &str, order_clause: &str) -> String {
    format!(
        "{}WHERE sl.publish_status = 'published' {category_filter} \
         ORDER BY {order_clause} LIMIT $1 OFFSET $2",
        agent_with_listing_select!()
    )
}

/// The cursor page of published agents: `after` is one of the `*_AFTER`
/// boundaries (or empty for the first page), `order` its `*_ORDER`, and
/// `limit_param` the number of the placeholder that carries the limit,
/// which follows the boundary's own.
pub(crate) fn published_page_sql(
    category_filter: &str,
    after: &str,
    order: &str,
    limit_param: u8,
) -> String {
    format!(
        "{}WHERE sl.publish_status = 'published' {category_filter} {after} \
         ORDER BY {order} LIMIT ${limit_param}",
        agent_with_listing_select!()
    )
}

/// The `%query%` pattern for a free-text search.
pub(crate) fn contains_pattern(query: &str) -> String {
    format!("%{query}%")
}

/// A page limit clamped to the Store's maximum, falling to `default` when
/// the caller left it unset.
pub(crate) fn page_limit(limit: Option<u32>, default: u32) -> i64 {
    i64::from(limit.unwrap_or(default).min(100))
}

/// A page offset, zero when the caller left it unset.
pub(crate) fn page_offset(offset: Option<u32>) -> i64 {
    i64::from(offset.unwrap_or(0))
}

/// A token count as the `int4` both `token_count` columns hold.
///
/// # Errors
/// Returns `AppError::invalid_input` when the count does not fit the column.
pub(crate) fn token_count_column(count: u32) -> AppResult<i32> {
    i32::try_from(count).map_err(|_| AppError::invalid_input("token_count out of range"))
}

/// Upper bound on numbered candidates tried before giving up on a title.
pub(crate) const MAX_HANDLE_ATTEMPTS: u32 = 100;

// ============================================================================
// Row parsers
// ============================================================================

fn listing_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read store listing column {name}: {e}"))
}

/// Read one column, naming it in the error.
pub(crate) fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    T: Decode<'r, R::Database> + Type<R::Database>,
{
    row.try_get(name).map_err(|e| listing_column_error(name, e))
}

/// The listing columns whose names depend on the statement: bare in a read
/// of `store_listings`, prefixed `sl_` in a join where the agent's own `id`,
/// `created_at` and `updated_at` take the bare names.
struct ListingColumnNames {
    id: &'static str,
    created_at: &'static str,
    updated_at: &'static str,
}

const BARE_LISTING_COLUMNS: ListingColumnNames = ListingColumnNames {
    id: "id",
    created_at: "created_at",
    updated_at: "updated_at",
};

const JOINED_LISTING_COLUMNS: ListingColumnNames = ListingColumnNames {
    id: "sl_id",
    created_at: "sl_created_at",
    updated_at: "sl_updated_at",
};

/// The listing in a row, its agent and tenant handed in by the caller that
/// knows which columns carry them. Timestamps decode as `DateTime<Utc>` and
/// `install_count` as `i32` on both drivers.
fn listing_from_row<'r, R>(
    row: &'r R,
    names: &ListingColumnNames,
    agent_id: Uuid,
    tenant_id: String,
) -> AppResult<StoreListing>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    let id: String = column(row, names.id)?;
    let publish_status: String = column(row, "publish_status")?;
    let install_count: i32 = column(row, "install_count")?;
    Ok(StoreListing {
        id: Uuid::parse_str(&id).map_err(|e| listing_column_error(names.id, e))?,
        agent_id,
        tenant_id,
        publish_status: PublishStatus::parse(&publish_status),
        published_at: column(row, "published_at")?,
        review_submitted_at: column(row, "review_submitted_at")?,
        review_decision_at: column(row, "review_decision_at")?,
        review_decision_by: column(row, "review_decision_by")?,
        rejection_reason: column(row, "rejection_reason")?,
        install_count: u32::try_from(install_count)
            .map_err(|e| listing_column_error("install_count", e))?,
        icon_url: column(row, "icon_url")?,
        author_id: column(row, "author_id")?,
        created_at: column(row, names.created_at)?,
        updated_at: column(row, names.updated_at)?,
    })
}

/// Convert a `store_listings` row to a [`StoreListing`].
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn store_listing_from_row<'r, R>(row: &'r R) -> AppResult<StoreListing>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    let agent_id: String = column(row, "agent_id")?;
    let agent_id = Uuid::parse_str(&agent_id).map_err(|e| listing_column_error("agent_id", e))?;
    let tenant_id: String = column(row, "tenant_id")?;
    listing_from_row(row, &BARE_LISTING_COLUMNS, agent_id, tenant_id)
}

/// Convert a joined agent-and-listing row to an [`AgentWithListing`].
/// `agent_row` is the backend's agent parser: the two differ in how
/// `user_id`, `tenant_id`, `is_system` and the timestamps decode, which is
/// the `agents` pair's seam, not this one's.
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn agent_with_listing_from_row<'r, R>(
    row: &'r R,
    agent_row: fn(&'r R) -> AppResult<Agent>,
) -> AppResult<AgentWithListing>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    let agent = agent_row(row)?;
    let listing = listing_from_row(
        row,
        &JOINED_LISTING_COLUMNS,
        agent.id,
        agent.tenant_id.clone(),
    )?;
    Ok(AgentWithListing { agent, listing })
}

/// Store admin statistics from the counts row.
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn store_admin_stats_from_row<'r, R>(row: &'r R) -> AppResult<StoreAdminStats>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    i64: Decode<'r, R::Database> + Type<R::Database>,
{
    let pending_count: i64 = column(row, "pending_count")?;
    let published_count: i64 = column(row, "published_count")?;
    let rejected_count: i64 = column(row, "rejected_count")?;
    let total_installs: i64 = column(row, "total_installs")?;

    let total_decided = published_count + rejected_count;
    #[allow(clippy::cast_precision_loss)]
    let rejection_rate = if total_decided > 0 {
        (rejected_count as f64 / total_decided as f64) * 100.0
    } else {
        0.0
    };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(StoreAdminStats {
        pending_count: pending_count as u32,
        published_count: published_count as u32,
        rejected_count: rejected_count as u32,
        total_installs: total_installs as u32,
        rejection_rate,
    })
}

/// Give an agent its catalogue handle, if it does not own one yet — the agent
/// being approved into the Store, or the one `/agent create` just created.
///
/// An origin agent that already carries a handle (a seeded agent) keeps it.
/// Any other agent — a custom agent with no handle, or a copy that inherited
/// its origin's handle as a reference — is assigned the first free candidate
/// derived from its title (`title`, then `title-2`, `title-3`, …). "Free"
/// is judged at catalogue scope: no origin agent and no published agent may
/// already answer to it. Origin rows are additionally guarded by the
/// `idx_agents_handle` unique index, so a concurrent approval of the same
/// candidate fails loudly on the second `UPDATE` instead of producing twins.
///
/// An expression over `$conn`, the approval transaction or a plain pool
/// connection, so the driver is the one the caller already holds; it
/// evaluates to the handle, or to the error that ends the caller's
/// transaction.
macro_rules! ensure_catalogue_handle {
    ($conn:expr, $agent_id:expr, $tenant_id:expr) => {{
        let agent_id: &str = $agent_id;
        let row = sqlx::query(HANDLE_OWNER_SQL)
            .bind(agent_id)
            .bind($tenant_id)
            .fetch_optional(&mut *$conn)
            .await
            .map_err(|e| AppError::database(format!("Failed to read coach for handle: {e}")))?
            .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;
        let title: String = column(&row, "title")?;
        let owned: Option<String> = column(&row, "slug")?;
        let forked_from: Option<String> = column(&row, "forked_from")?;
        if let (Some(handle), None) = (owned, forked_from) {
            Ok(handle)
        } else {
            let base = AgentHandle::derive(&title);
            let mut assigned = None;
            for attempt in 0..MAX_HANDLE_ATTEMPTS {
                let candidate = base.candidate(attempt);
                let taken = sqlx::query(HANDLE_TAKEN_SQL)
                    .bind(candidate.as_str())
                    .bind(agent_id)
                    .fetch_optional(&mut *$conn)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to probe coach handle: {e}"))
                    })?;
                if taken.is_some() {
                    continue;
                }
                sqlx::query(ASSIGN_HANDLE_SQL)
                    .bind(candidate.as_str())
                    .bind(agent_id)
                    .execute(&mut *$conn)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to assign coach handle: {e}"))
                    })?;
                assigned = Some(candidate.as_str().to_owned());
                break;
            }
            assigned.ok_or_else(|| {
                AppError::invalid_input(format!(
                    "No free catalogue handle derived from '{title}' after {MAX_HANDLE_ATTEMPTS} candidates"
                ))
            })
        }
    }};
}
pub(crate) use ensure_catalogue_handle;
