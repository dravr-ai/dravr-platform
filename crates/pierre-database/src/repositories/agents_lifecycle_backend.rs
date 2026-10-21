// ABOUTME: The agent lifecycle writes of AgentsRepository, emitted per backend by macro: fork-on-edit and delete
// ABOUTME: Invoked by each backend shell beside impl_agents_repository, which delegates fork_agent and delete here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The two multi-statement agent writes (carnet#1096), written once.
//!
//! [`super::agents_backend`]'s `AgentsRepository` impl delegates `fork_agent`
//! and `delete` to the inherent methods this macro emits. Like that macro,
//! the body names its consts, helpers and types unqualified, so the invoking
//! shell must `use` every one of them.

/// Emit the fork and delete bodies for one backend type.
///
/// `$ids` is the backend's uuid codec (`TextUuid` on `SQLite`, `NativeUuid`
/// on Postgres).
macro_rules! impl_agent_lifecycle {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// The body of [`AgentsRepository::fork_agent`].
            async fn fork_into_copy(
                &self,
                origin_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Agent> {
                let origin = AgentsRepository::get_by_id(self, origin_id, user_id, tenant_id)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("Coach {origin_id}")))?;
                // Their own agent, an edited copy included, is already theirs to edit.
                if !origin.is_system && origin.user_id == user_id {
                    return Ok(origin);
                }
                if let Some(copy) = self
                    .fetch_one_agent(
                        sqlx::query(OWNED_COPY_OF_SQL)
                            .bind($ids::bind(user_id))
                            .bind(tenant_id)
                            .bind(origin_id),
                        "find forked coach",
                    )
                    .await?
                {
                    return Ok(copy);
                }

                let copy_id = Uuid::new_v4().to_string();
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to begin fork: {e}")))?;
                let forked = sqlx::query(FORK_AGENT_SQL)
                    .bind(&copy_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(Utc::now())
                    .bind(AgentVisibility::Private.as_str())
                    .bind(origin_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to fork coach: {e}")))?;
                if forked.rows_affected() == 0 {
                    // Visible through an assignment but not reachable from
                    // this tenant: nothing to fork here.
                    return Err(AppError::not_found(format!("Coach {origin_id}")));
                }
                sqlx::query(FORK_TRANSLATIONS_SQL)
                    .bind(&copy_id)
                    .bind(origin_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fork coach translations: {e}"))
                    })?;
                sqlx::query(COUNT_FORK_AS_INSTALL_SQL)
                    .bind(Utc::now())
                    .bind(origin_id)
                    .bind($ids::bind(user_id))
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count fork: {e}")))?;
                // The user's row — membership, counters, favourite — follows
                // them onto the copy; a user who held none gets a membership.
                let moved = sqlx::query(MOVE_MEMBERSHIP_SQL)
                    .bind(&copy_id)
                    .bind(origin_id)
                    .bind($ids::bind(user_id))
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to move membership: {e}")))?;
                if moved.rows_affected() == 0 {
                    sqlx::query(ENSURE_ASSIGNMENT_SQL)
                        .bind(Uuid::new_v4().to_string())
                        .bind(&copy_id)
                        .bind($ids::bind(user_id))
                        .bind(Utc::now())
                        .bind(true)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to create membership: {e}"))
                        })?;
                }
                sqlx::query(RESELECT_FORK_SQL)
                    .bind(&copy_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(origin_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to move selection: {e}")))?;
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to commit fork: {e}")))?;

                AgentsRepository::get_by_id(self, &copy_id, user_id, tenant_id)
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("Coach {copy_id}")))
            }

            /// The body of [`AgentsRepository::delete`]: what ran on the agent is
            /// handed to its origin (or let go of) before the row goes, in one
            /// transaction.
            async fn delete_owned_agent(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let owned = sqlx::query(OWNED_AGENT_FORKED_FROM_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?;
                let Some(row) = owned else {
                    return Ok(false);
                };
                let origin: Option<String> = column(&row, "forked_from")?;

                // What ran on the agent moves to its origin, or lets go of it.
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to begin delete: {e}")))?;
                sqlx::query(HAND_ON_AGENT_CONVERSATIONS_SQL)
                    .bind(agent_id)
                    .bind(origin.as_deref())
                    .bind(tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to detach coach conversations: {e}"))
                    })?;
                sqlx::query(REPOINT_AGENT_GROUPS_SQL)
                    .bind(agent_id)
                    .bind(origin.as_deref())
                    .bind(tenant_id.to_string())
                    .bind(tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to re-point coach groups: {e}"))
                    })?;
                let stranded = sqlx::query(AGENT_GROUPS_LEFT_SQL)
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check coach groups: {e}"))
                    })?;
                if stranded.is_some() {
                    // Dropping the transaction rolls the detach back.
                    return Err(AppError::invalid_input(
                        "A coaching group still runs on this agent: move the group to another \
                         agent first",
                    ));
                }
                let result = sqlx::query(DELETE_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to delete coach: {e}")))?;
                // A copy counted as one install of its origin.
                if let Some(origin) = origin.as_deref() {
                    sqlx::query(DECREMENT_INSTALLS_SQL)
                        .bind(Utc::now())
                        .bind(origin)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to decrement install count: {e}"))
                        })?;
                }
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to commit delete: {e}")))?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_agent_lifecycle;
