// ABOUTME: The inherent helpers AgentsRepository's bodies share, emitted per backend by macro: membership, access, fetch
// ABOUTME: Invoked by each backend shell beside impl_agents_repository and impl_agent_lifecycle, which both call these
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The helpers behind the agent repository bodies, written once.
//!
//! [`super::agents_backend`] and [`super::agents_lifecycle_backend`] call the
//! inherent methods this macro emits: install counting on a membership change,
//! the ownership, reach and hide predicates, the one-agent fetch, and the
//! translation overlays. Like those macros, the body names its consts, helpers
//! and types unqualified, so the invoking shell must `use` every one of them.

/// Emit the shared inherent helpers for one backend type.
///
/// `$row` is the driver's row type and `$ids` the backend's uuid codec
/// (`SqliteRow` + `TextUuid`, `PgRow` + `NativeUuid`).
macro_rules! impl_agents_helpers {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// Make sure the user holds a row for the agent; `member` makes
            /// it a team membership (an existing usage row is promoted,
            /// counters kept), while `false` only ensures a usage row exists
            /// and never demotes a membership. `true` when a row was created
            /// or just became a membership. A row that just became a
            /// membership is one more install of the agent's published
            /// listing, as a Store install is.
            async fn ensure_agent_assignment(
                &self,
                agent_id: &str,
                user_id: Uuid,
                member: bool,
            ) -> AppResult<bool> {
                let result = sqlx::query(ENSURE_ASSIGNMENT_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(Utc::now())
                    .bind(member)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to ensure coach assignment: {e}"))
                    })?;
                let changed = result.rows_affected() > 0;
                if member && changed {
                    self.count_membership_change(agent_id, true).await?;
                }
                Ok(changed)
            }

            /// Keep the agent's listing `install_count` in step with its
            /// team: one install per membership, whichever path made it (a
            /// Store install, an admin assignment, activating or
            /// favouriting), and one fewer when a membership ends, so a
            /// later Store uninstall never takes away an install nobody
            /// counted. Only a published listing counts up, as a Store
            /// install does; the decrement is clamped at zero.
            async fn count_membership_change(&self, agent_id: &str, joined: bool) -> AppResult<()> {
                let sql = if joined {
                    INCREMENT_INSTALLS_SQL
                } else {
                    DECREMENT_INSTALLS_SQL
                };
                sqlx::query(sql)
                    .bind(Utc::now())
                    .bind(agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update install count: {e}"))
                    })?;
                Ok(())
            }

            /// Whether the user owns the agent in the tenant — the predicate
            /// `UPDATE_AGENT_SQL` writes under.
            async fn agent_owned(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                Ok(sqlx::query(AGENT_OWNED_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?
                    .is_some())
            }

            /// Whether the caller can reach the agent (see `AGENT_REACHABLE_SQL`).
            async fn agent_reachable(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                Ok(sqlx::query(AGENT_REACHABLE_SQL)
                    .bind(agent_id)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?
                    .is_some())
            }

            /// Whether the agent belongs to the tenant; a not-found error otherwise.
            async fn require_agent_in_tenant(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<()> {
                let exists = sqlx::query(AGENT_IN_TENANT_EXISTS_SQL)
                    .bind(agent_id)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
                if exists.is_none() {
                    return Err(AppError::not_found(format!("Coach {agent_id}")));
                }
                Ok(())
            }

            /// Whether the user may hide the agent: a system agent, or one of
            /// their tenant assigned to them — never their own agent.
            async fn agent_hideable(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let is_system = sqlx::query(AGENT_IS_SYSTEM_SQL)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to check system agent: {e}")))?
                    .is_some();
                if is_system {
                    return Ok(true);
                }
                Ok(sqlx::query(AGENT_ASSIGNED_IN_TENANT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?
                    .is_some())
            }

            /// The one agent a bound read returns, if any.
            async fn fetch_one_agent<'q>(
                &self,
                query: sqlx::query::Query<
                    'q,
                    <$row as sqlx::Row>::Database,
                    <<$row as sqlx::Row>::Database as sqlx::Database>::Arguments<'q>,
                >,
                what: &str,
            ) -> AppResult<Option<Agent>> {
                let row = query
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to {what}: {e}")))?;
                row.as_ref().map(row_to_agent).transpose()
            }

            /// Stored text of one nullable agent column, kept by an update
            /// that does not name it.
            async fn stored_agent_text(
                &self,
                sql: &str,
                agent_id: &str,
                what: &str,
            ) -> AppResult<Option<String>> {
                let row: Option<(Option<String>,)> = sqlx::query_as(sql)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get {what}: {e}")))?;
                Ok(row.and_then(|(value,)| value))
            }

            /// The overlays translating these agents into `locale`; none for
            /// English, which is what the rows are written in.
            async fn agent_translation_overlays(
                &self,
                ids: &[String],
                locale: &str,
            ) -> AppResult<HashMap<String, AgentFieldOverlay>> {
                let mut overlays = HashMap::new();
                if locale == "en" || ids.is_empty() {
                    return Ok(overlays);
                }
                let sql = translation_overlays_sql(ids.len());
                let mut query = sqlx::query(&sql).bind(locale);
                for id in ids {
                    query = query.bind(id);
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to load coach translations: {e}"))
                })?;
                for row in &rows {
                    let id: String = column(row, "agent_id")?;
                    overlays.insert(
                        id,
                        AgentFieldOverlay {
                            title: row.try_get("title").ok(),
                            description: row.try_get("description").ok(),
                            purpose: row.try_get("purpose").ok(),
                            instructions: row.try_get("instructions").ok(),
                            tags: row
                                .try_get::<Option<String>, _>("tags")
                                .ok()
                                .flatten()
                                .and_then(|raw| serde_json::from_str(&raw).ok()),
                            sample_prompts: row
                                .try_get::<Option<String>, _>("sample_prompts")
                                .ok()
                                .flatten()
                                .and_then(|raw| serde_json::from_str(&raw).ok()),
                        },
                    );
                }
                Ok(overlays)
            }
        }
    };
}
pub(crate) use impl_agents_helpers;
