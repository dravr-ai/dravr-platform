// ABOUTME: The one body of ActivityBackfillJobRepository, emitted for each backend type by its shell
// ABOUTME: sqlx resolves the driver from self.pool(); each shell passes its type and its claim lock clause

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

macro_rules! impl_activity_backfill_job_repository {
    ($ty:ty, $lock:literal) => {
        #[async_trait::async_trait]
        impl ActivityBackfillJobRepository for $ty {
            async fn record_backfill_job(&self, row: &ActivityBackfillJobRow) -> AppResult<bool> {
                let live_read = row.kind == BackfillJobKind::LiveRead;
                let sql = if live_read {
                    RECORD_LIVE_READ_SQL
                } else {
                    RECORD_BACKFILL_JOB_SQL
                };
                let query = sqlx::query(sql)
                    .bind(&row.id)
                    .bind(row.tenant_id)
                    .bind(row.user_id.to_string())
                    .bind(&row.provider)
                    .bind(row.after_ts)
                    .bind(row.before_ts)
                    .bind(row.fetch_limit)
                    .bind(&row.conversation_id)
                    .bind(row.created_at_ms)
                    .bind(row.leased_until_ms)
                    .bind(row.attempts)
                    .bind(&row.lease_token)
                    .bind(row.kind.as_str())
                    .bind(row.progress.units_total)
                    .bind(row.progress.units_done)
                    .bind(row.progress.rows_landed)
                    .bind(row.progress.secs_per_unit)
                    .bind(&row.turn_id)
                    .bind(&row.asked_message_id)
                    .bind(&row.walk_sport);
                let query = if live_read {
                    query
                } else {
                    query.bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                };
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to record backfill job: {e}"))
                };
                if live_read {
                    let written = query.fetch_optional(self.pool()).await.map_err(failed)?;
                    let written_id: Option<String> = written
                        .map(|written| written.try_get("id"))
                        .transpose()
                        .map_err(failed)?;
                    return Ok(written_id.as_deref() == Some(row.id.as_str()));
                }
                let result = query.execute(self.pool()).await.map_err(failed)?;
                Ok(result.rows_affected() == 1)
            }

            async fn rebind_live_read(&self, ask: &ActivityBackfillJobRow) -> AppResult<bool> {
                let rebound =
                    sqlx::query($crate::repositories::activity_backfill_jobs::REBIND_LIVE_READ_SQL)
                        .bind(ask.user_id.to_string())
                        .bind(ask.tenant_id)
                        .bind(&ask.provider)
                        .bind(BackfillJobKind::LiveRead.as_str())
                        .bind(&ask.conversation_id)
                        .bind(&ask.turn_id)
                        .bind(&ask.asked_message_id)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to rebind live read: {e}"))
                        })?;
                Ok(rebound.rows_affected() == 1)
            }

            async fn claim_stale_backfill_jobs(
                &self,
                claim: BackfillJobClaim,
            ) -> AppResult<Vec<ActivityBackfillJobRow>> {
                let rows = sqlx::query(claim_backfill_jobs_sql!($lock))
                    .bind(claim.now_ms.saturating_add(claim.lease_ms))
                    .bind(claim.now_ms)
                    .bind(claim.now_ms.saturating_sub(claim.queued_older_than_ms))
                    .bind(claim.max_attempts)
                    .bind(claim.limit)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim backfill jobs: {e}"))
                    })?;
                rows.iter().map(backfill_job_from_row).collect()
            }

            async fn renew_backfill_job_lease(
                &self,
                lease: &JobLease,
                leased_until_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(RENEW_BACKFILL_JOB_LEASE_SQL)
                    .bind(leased_until_ms)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to renew backfill job lease: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_backfill_job(&self, lease: &JobLease) -> AppResult<bool> {
                let done = sqlx::query(FINISH_BACKFILL_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish backfill job: {e}"))
                    })?;
                Ok(done.rows_affected() == 1)
            }

            async fn finish_live_read(
                &self,
                lease: &JobLease,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::LiveReadAsk>> {
                let failed =
                    |e: sqlx::Error| AppError::database(format!("Failed to finish live read: {e}"));
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::FINISH_LIVE_READ_SQL)
                        .bind(&lease.id)
                        .bind(&lease.token)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    Ok($crate::repositories::activity_backfill_jobs::LiveReadAsk {
                        conversation_id: row.try_get("conversation_id").map_err(failed)?,
                        turn_id: row.try_get("turn_id").map_err(failed)?,
                        asked_message_id: row.try_get("asked_message_id").map_err(failed)?,
                    })
                })
                .transpose()
            }

            async fn end_backfill_job_incomplete(&self, lease: &JobLease) -> AppResult<bool> {
                let ended = sqlx::query(END_BACKFILL_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(incomplete_outcome())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to end backfill job: {e}")))?;
                Ok(ended.rows_affected() == 1)
            }

            async fn release_backfill_job_claim(
                &self,
                lease: &JobLease,
                pause_until_ms: Option<i64>,
            ) -> AppResult<bool> {
                let released = sqlx::query(RELEASE_BACKFILL_JOB_CLAIM_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(pause_until_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release backfill job claim: {e}"))
                    })?;
                Ok(released.rows_affected() == 1)
            }

            async fn release_backfill_job_lease(
                &self,
                lease: &JobLease,
                retry_at_ms: i64,
            ) -> AppResult<bool> {
                let released = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::RELEASE_BACKFILL_JOB_LEASE_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(retry_at_ms)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to release backfill job lease: {e}"))
                })?;
                Ok(released.rows_affected() == 1)
            }

            async fn take_over_idle_live_read(
                &self,
                ask: &ActivityBackfillJobRow,
            ) -> AppResult<Option<(JobLease, i64)>> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to take over an idle live read: {e}"))
                };
                let Some(token) = ask.lease_token.clone() else {
                    return Ok(None);
                };
                let row = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::TAKE_OVER_IDLE_LIVE_READ_SQL,
                )
                .bind(ask.user_id.to_string())
                .bind(ask.tenant_id)
                .bind(&ask.provider)
                .bind(BackfillJobKind::LiveRead.as_str())
                .bind(ask.after_ts)
                .bind(ask.before_ts)
                .bind(ask.fetch_limit)
                .bind(&ask.conversation_id)
                .bind(&ask.turn_id)
                .bind(&ask.asked_message_id)
                .bind(ask.leased_until_ms)
                .bind(&token)
                .bind(ask.created_at_ms)
                .fetch_optional(self.pool())
                .await
                .map_err(failed)?;
                row.map(|row| {
                    let id: String = row.try_get("id").map_err(failed)?;
                    let attempts: i64 = row.try_get("attempts").map_err(failed)?;
                    Ok((JobLease { id, token }, attempts))
                })
                .transpose()
            }

            async fn backfill_job_lease_state(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
                now_ms: i64,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::JobLeaseState>>
            {
                use $crate::repositories::activity_backfill_jobs::JobLeaseState;
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read backfill job lease: {e}"))
                };
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::BACKFILL_JOB_LEASE_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(kind.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    let leased_until_ms: i64 = row.try_get("leased_until_ms").map_err(failed)?;
                    Ok(if leased_until_ms > now_ms {
                        JobLeaseState::Leased
                    } else {
                        JobLeaseState::Idle
                    })
                })
                .transpose()
            }

            async fn history_walk_row(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::HistoryWalkRowState>>
            {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read history walk row: {e}"))
                };
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::HISTORY_WALK_ROW_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(BackfillJobKind::HistoryWalk.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(failed)?;
                    Ok($crate::repositories::activity_backfill_jobs::HistoryWalkRowState {
                        state: backfill_job_state_of(outcome.as_deref())?,
                        walk_backend: row.try_get("walk_backend").map_err(failed)?,
                    })
                })
                .transpose()
            }

            async fn leased_walk_row(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                lease: &JobLease,
                kind: BackfillJobKind,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::LeasedWalkRow>>
            {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read a leased walk row: {e}"))
                };
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::LEASED_WALK_ROW_SQL)
                        .bind(&lease.id)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(kind.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(failed)?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(failed)?;
                    let token: Option<String> = row.try_get("lease_token").map_err(failed)?;
                    Ok($crate::repositories::activity_backfill_jobs::LeasedWalkRow {
                        provider: row.try_get("provider").map_err(failed)?,
                        state: backfill_job_state_of(outcome.as_deref())?,
                        walk_backend: row.try_get("walk_backend").map_err(failed)?,
                        leased_here: token.as_deref() == Some(lease.token.as_str()),
                    })
                })
                .transpose()
            }

            async fn record_job_progress(
                &self,
                lease: &JobLease,
                step: JobProgressStep,
            ) -> AppResult<bool> {
                let recorded = sqlx::query(RECORD_JOB_PROGRESS_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(step.before_ts)
                    .bind(step.units_done)
                    .bind(step.rows_landed)
                    .bind(step.secs_per_unit)
                    .bind(step.floor.map(|floor| floor.after_ts))
                    .bind(step.floor.and_then(|floor| floor.units_total))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record backfill job progress: {e}"))
                    })?;
                Ok(recorded.rows_affected() == 1)
            }

            async fn widen_detail_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                widen: &$crate::repositories::activity_backfill_jobs::WalkWiden,
            ) -> AppResult<bool> {
                let widened =
                    sqlx::query($crate::repositories::activity_backfill_jobs::WIDEN_DETAIL_WALK_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(BackfillJobKind::DetailWalk.as_str())
                        .bind(widen.after_ts)
                        .bind(widen.units_left)
                        .bind(&widen.walk_sport)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to widen a detail walk: {e}"))
                        })?;
                Ok(widened.rows_affected() == 1)
            }

            async fn end_detail_walk(
                &self,
                lease: &JobLease,
                end: &$crate::repositories::activity_backfill_jobs::DetailWalkEnd,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::DetailWalkEnded>>
            {
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::END_DETAIL_WALK_SQL)
                        .bind(&lease.id)
                        .bind(&lease.token)
                        .bind(end.before_ts)
                        .bind(end.units_done)
                        .bind(end.rows_landed)
                        .bind(end.secs_per_unit)
                        .bind(end.walked_floor)
                        .bind(walked_outcome())
                        .bind(end.ended_at_ms)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to end a detail walk: {e}"))
                        })?;
                row.as_ref()
                    .map($crate::repositories::activity_backfill_jobs::detail_walk_ended_from_row)
                    .transpose()
            }

            async fn credit_detail_walk(
                &self,
                row_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
                asked_at_ms: i64,
                stored: i64,
            ) -> AppResult<bool> {
                let credited = sqlx::query($crate::repositories::activity_backfill_jobs::CREDIT_DETAIL_WALK_SQL)
                    .bind(row_id)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(BackfillJobKind::DetailWalk.as_str())
                    .bind(stored)
                    .bind(asked_at_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to credit a detail walk: {e}")))?;
                Ok(credited.rows_affected() == 1)
            }

            async fn set_detail_walk_tried(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                tried: Option<&$crate::repositories::activity_backfill_jobs::WalkTried>,
            ) -> AppResult<bool> {
                let set = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::SET_DETAIL_WALK_TRIED_SQL,
                )
                .bind(user_id.to_string())
                .bind(tenant_id)
                .bind(provider)
                .bind(BackfillJobKind::DetailWalk.as_str())
                .bind(tried.and_then(|tried| tried.floor_ts))
                .bind(tried.map(|tried| tried.top_ts))
                .bind(tried.map(|tried| tried.asked_ms))
                .bind(tried.map(|tried| tried.ended_ms))
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to record a detail walk's tried range: {e}"))
                })?;
                Ok(set.rows_affected() == 1)
            }

            async fn detail_walk_row(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
            ) -> AppResult<Option<$crate::repositories::activity_backfill_jobs::DetailWalkRow>>
            {
                let row =
                    sqlx::query($crate::repositories::activity_backfill_jobs::DETAIL_WALK_ROW_SQL)
                        .bind(user_id.to_string())
                        .bind(tenant_id)
                        .bind(provider)
                        .bind(kind.as_str())
                        .fetch_optional(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to read a detail walk row: {e}"))
                        })?;
                row.as_ref()
                    .map($crate::repositories::activity_backfill_jobs::detail_walk_row_from_row)
                    .transpose()
            }

            async fn end_history_walk(
                &self,
                lease: &JobLease,
                step: JobProgressStep,
            ) -> AppResult<Option<JobProgress>> {
                let row = sqlx::query(END_HISTORY_WALK_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(step.before_ts)
                    .bind(step.units_done)
                    .bind(step.rows_landed)
                    .bind(step.secs_per_unit)
                    .bind(step.floor.map(|floor| floor.after_ts))
                    .bind(step.floor.and_then(|floor| floor.units_total))
                    .bind(walked_outcome())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to end history walk: {e}")))?;
                row.as_ref().map(job_progress_from_row).transpose()
            }

            async fn hand_off_history_walk(
                &self,
                lease: &JobLease,
                hand_off: &$crate::repositories::activity_backfill_jobs::WalkHandOff,
            ) -> AppResult<bool> {
                let handed = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::HAND_OFF_HISTORY_WALK_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(&hand_off.backend)
                .bind(hand_off.before_ts)
                .bind(hand_off.units_done)
                .bind(hand_off.rows_landed)
                .bind(hand_off.fetch_limit)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to hand off history walk: {e}")))?;
                Ok(handed.rows_affected() == 1)
            }

            async fn reopen_history_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                from: $crate::repositories::activity_backfill_jobs::ReopenFrom,
                hand_off: &$crate::repositories::activity_backfill_jobs::WalkHandOff,
                now_ms: i64,
            ) -> AppResult<bool> {
                let reopened = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::REOPEN_HISTORY_WALK_SQL,
                )
                .bind(user_id.to_string())
                .bind(tenant_id)
                .bind(provider)
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind(&hand_off.backend)
                .bind(hand_off.before_ts)
                .bind(hand_off.units_done)
                .bind(hand_off.fetch_limit)
                .bind(from.outcome())
                .bind(now_ms)
                .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to reopen history walk: {e}")))?;
                Ok(reopened.rows_affected() == 1)
            }

            async fn fail_history_walk(
                &self,
                lease: &JobLease,
                ended_at_ms: i64,
            ) -> AppResult<bool> {
                let ended =
                    sqlx::query($crate::repositories::activity_backfill_jobs::FAIL_HISTORY_WALK_SQL)
                        .bind(&lease.id)
                        .bind(&lease.token)
                        .bind(incomplete_outcome())
                        .bind(ended_at_ms)
                        .execute(self.pool())
                        .await
                        .map_err(|e| AppError::database(format!("Failed to end history walk: {e}")))?;
                Ok(ended.rows_affected() == 1)
            }

            async fn resume_incomplete_history_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                ended_by_ms: i64,
            ) -> AppResult<bool> {
                let resumed = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::RESUME_INCOMPLETE_HISTORY_WALK_SQL,
                )
                .bind(user_id.to_string())
                .bind(tenant_id)
                .bind(provider)
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind(incomplete_outcome())
                .bind(ended_by_ms)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to resume history walk: {e}")))?;
                Ok(resumed.rows_affected() == 1)
            }

            async fn settle_purged_history_walk(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                now_ms: i64,
            ) -> AppResult<bool> {
                let settled = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::SETTLE_PURGED_HISTORY_WALK_SQL,
                )
                .bind(user_id.to_string())
                .bind(tenant_id)
                .bind(provider)
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind(walked_outcome())
                .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                .bind(now_ms)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to settle a purged history walk: {e}"))
                })?;
                Ok(settled.rows_affected() == 1)
            }

            async fn purged_history_walks(
                &self,
                now_ms: i64,
                limit: i64,
            ) -> AppResult<Vec<$crate::repositories::activity_backfill_jobs::PurgedHistoryWalk>>
            {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read purged history walks: {e}"))
                };
                let rows = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::PURGED_HISTORY_WALKS_SQL,
                )
                .bind(BackfillJobKind::HistoryWalk.as_str())
                .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                .bind(now_ms)
                .bind(limit)
                .fetch_all(self.pool())
                .await
                .map_err(failed)?;
                rows.iter()
                    .map(|row| {
                        let user_id: String = row.try_get("user_id").map_err(failed)?;
                        Ok($crate::repositories::activity_backfill_jobs::PurgedHistoryWalk {
                            user_id: Uuid::parse_str(&user_id).map_err(|e| {
                                AppError::database(format!(
                                    "purged history walk user_id is not a UUID: {e}"
                                ))
                            })?,
                            tenant_id: row.try_get("tenant_id").map_err(failed)?,
                            provider: row.try_get("provider").map_err(failed)?,
                        })
                    })
                    .collect()
            }

            async fn job_progress(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
            ) -> AppResult<Option<JobProgress>> {
                let row = sqlx::query(JOB_PROGRESS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(provider)
                    .bind(kind.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read backfill job progress: {e}"))
                    })?;
                row.as_ref().map(job_progress_from_row).transpose()
            }

            async fn backfill_job_state(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                kind: BackfillJobKind,
            ) -> AppResult<Option<BackfillJobState>> {
                let row = sqlx::query(BACKFILL_JOB_STATE_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(provider)
                    .bind(kind.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read backfill job state: {e}"))
                    })?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(|e| {
                        AppError::database(format!("activity_backfill_jobs outcome: {e}"))
                    })?;
                    backfill_job_state_of(outcome.as_deref())
                })
                .transpose()
            }

            async fn reap_exhausted_backfill_jobs(
                &self,
                now_ms: i64,
                max_attempts: i64,
            ) -> AppResult<Vec<ActivityBackfillJobRow>> {
                let rows = sqlx::query(REAP_EXHAUSTED_BACKFILL_JOBS_SQL)
                    .bind(max_attempts)
                    .bind(now_ms)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to reap exhausted backfill jobs: {e}"))
                    })?;
                rows.iter().map(backfill_job_from_row).collect()
            }

            async fn wellness_resume_day(
                &self,
                lease: &JobLease,
                tenant_id: TenantId,
            ) -> AppResult<Option<dravr_tronc::dates::CivilDate>> {
                use sqlx::Row as _;
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read the wellness resume day: {e}"))
                };
                let row = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::WELLNESS_RESUME_DAY_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(tenant_id)
                .fetch_optional(self.pool())
                .await
                .map_err(failed)?;
                let day: Option<String> = row
                    .map(|row| row.try_get("wellness_resume_day"))
                    .transpose()
                    .map_err(failed)?
                    .flatten();
                day.map(|day| {
                    day.parse::<dravr_tronc::dates::CivilDate>().map_err(|e| {
                        AppError::database(format!("The wellness resume day is not a date: {e}"))
                    })
                })
                .transpose()
            }

            async fn pause_wellness_copy(
                &self,
                lease: &JobLease,
                tenant_id: TenantId,
                resume_day: dravr_tronc::dates::CivilDate,
                pause_until_ms: i64,
            ) -> AppResult<bool> {
                let paused = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::PAUSE_WELLNESS_COPY_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(tenant_id)
                .bind(resume_day.to_string())
                .bind(pause_until_ms)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to pause the wellness copy: {e}")))?;
                Ok(paused.rows_affected() == 1)
            }

            async fn wellness_first_filled(
                &self,
                lease: &JobLease,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                use sqlx::Row as _;
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to read the wellness first fill: {e}"))
                };
                let row = sqlx::query($crate::repositories::activity_backfill_jobs::WELLNESS_FILLED_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(failed)?;
                let filled_at: Option<i64> = row
                    .map(|row| row.try_get("wellness_filled_at_ms"))
                    .transpose()
                    .map_err(failed)?
                    .flatten();
                Ok(filled_at.is_some())
            }

            async fn mark_wellness_first_filled(
                &self,
                lease: &JobLease,
                tenant_id: TenantId,
                filled_at_ms: i64,
            ) -> AppResult<bool> {
                let marked = sqlx::query(
                    $crate::repositories::activity_backfill_jobs::MARK_WELLNESS_FILLED_SQL,
                )
                .bind(&lease.id)
                .bind(&lease.token)
                .bind(tenant_id)
                .bind(filled_at_ms)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to record the wellness first fill: {e}"))
                })?;
                Ok(marked.rows_affected() == 1)
            }
        }
    };
}
pub(crate) use impl_activity_backfill_job_repository;
