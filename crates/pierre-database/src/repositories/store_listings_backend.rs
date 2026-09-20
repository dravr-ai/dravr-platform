// ABOUTME: The one StoreListingsRepository implementation, emitted per backend by impl_store_listings_repository!
// ABOUTME: The trait body over the statements and parsers in repositories/store_listings.rs; the shells are one line each
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Emit the whole [`StoreListingsRepository`](super::store_listings::StoreListingsRepository)
/// implementation for one backend type. The body is written once here; each
/// backend's shell invokes it with its own type, its uuid codec, its agent
/// row parser and its match operator, and sqlx resolves the driver from
/// `self.pool()` per expansion. The body names its consts, helpers and types
/// unqualified, so the invoking shell must `use` every one of them.
///
/// `$ids` is the codec in [`super::uuid_columns`] for how that backend's
/// `users.id`, `agents.user_id` and `agent_assignments.user_id` columns bind.
/// `$agent_row` is the backend's `agents` row parser. `$like` is the
/// backend's case-folding text match, spliced into the search: `"ILIKE"` on
/// Postgres, `"LIKE"` on `SQLite`. `SQLite`'s `LIKE` folds case for ASCII
/// letters only, so a search for a title with an accented letter is
/// case-sensitive there and not on Postgres; production runs Postgres, so no
/// athlete sees the narrower fold.
macro_rules! impl_store_listings_repository {
    ($ty:ty, $ids:ident, $agent_row:path, $like:literal) => {
        impl $ty {
            /// An agent with its listing, within the listing's tenant.
            async fn store_agent_with_listing(
                &self,
                agent_id: &str,
                tenant_id: &TenantId,
            ) -> AppResult<AgentWithListing> {
                let row = sqlx::query(AGENT_WITH_LISTING_SQL)
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get coach with listing: {e}"))
                    })?;
                agent_with_listing_from_row(&row, $agent_row)
            }

            /// One page of the published catalogue in `sort_by` order, from
            /// the row after `cursor`, `fetch_limit` rows at most.
            async fn store_published_page(
                &self,
                sort_by: StoreSortOrder,
                category_filter: &str,
                cursor: Option<&StoreCursor>,
                fetch_limit: i64,
            ) -> AppResult<Vec<AgentWithListing>> {
                let rows = match (sort_by, cursor) {
                    (StoreSortOrder::Newest, Some(c)) => {
                        sqlx::query(&published_page_sql(
                            category_filter,
                            NEWEST_AFTER,
                            NEWEST_ORDER,
                            3,
                        ))
                        .bind(c.published_at)
                        .bind(&c.id)
                        .bind(fetch_limit)
                        .fetch_all(self.pool())
                        .await
                    }
                    (StoreSortOrder::Newest, None) => {
                        sqlx::query(&published_page_sql(category_filter, "", NEWEST_ORDER, 1))
                            .bind(fetch_limit)
                            .fetch_all(self.pool())
                            .await
                    }
                    (StoreSortOrder::Popular, Some(c)) => {
                        sqlx::query(&published_page_sql(
                            category_filter,
                            POPULAR_AFTER,
                            POPULAR_ORDER,
                            4,
                        ))
                        .bind(i64::from(c.install_count.unwrap_or(0)))
                        .bind(c.published_at)
                        .bind(&c.id)
                        .bind(fetch_limit)
                        .fetch_all(self.pool())
                        .await
                    }
                    (StoreSortOrder::Popular, None) => {
                        sqlx::query(&published_page_sql(category_filter, "", POPULAR_ORDER, 1))
                            .bind(fetch_limit)
                            .fetch_all(self.pool())
                            .await
                    }
                    (StoreSortOrder::Title, Some(c)) => {
                        sqlx::query(&published_page_sql(
                            category_filter,
                            TITLE_AFTER,
                            TITLE_ORDER,
                            3,
                        ))
                        .bind(c.title.as_deref().unwrap_or(""))
                        .bind(&c.id)
                        .bind(fetch_limit)
                        .fetch_all(self.pool())
                        .await
                    }
                    (StoreSortOrder::Title, None) => {
                        sqlx::query(&published_page_sql(category_filter, "", TITLE_ORDER, 1))
                            .bind(fetch_limit)
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches ({sort_by:?}): {e}"))
                })?;
                rows.iter()
                    .map(|row| agent_with_listing_from_row(row, $agent_row))
                    .collect()
            }
        }

        #[async_trait::async_trait]
        impl StoreListingsRepository for $ty {
            async fn assign_catalogue_handle(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<String> {
                let mut conn = self.pool().acquire().await.map_err(|e| {
                    AppError::database(format!(
                        "Failed to acquire connection for coach handle: {e}"
                    ))
                })?;
                ensure_catalogue_handle!(conn, agent_id, tenant_id)
            }

            async fn submit_for_review(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<StoreListing> {
                let now = Utc::now();

                let agent_row = sqlx::query(OWNED_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check coach ownership: {e}"))
                    })?;

                if agent_row.is_none() {
                    return Err(AppError::invalid_input(
                        "Agent not found, not owned by you, or not in your tenant",
                    ));
                }

                let existing = sqlx::query(EXISTING_LISTING_SQL)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check existing listing: {e}"))
                    })?;

                if let Some(row) = existing {
                    let status: String = column(&row, "publish_status")?;
                    if status != "draft" {
                        return Err(AppError::invalid_input(
                            "Coach is not in draft status — cannot submit for review",
                        ));
                    }
                    let listing_id: String = column(&row, "id")?;

                    sqlx::query(SUBMIT_LISTING_SQL)
                        .bind(PublishStatus::PendingReview.as_str())
                        .bind(now)
                        .bind(&listing_id)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to submit for review: {e}"))
                        })?;

                    sqlx::query(TOUCH_AGENT_SQL)
                        .bind(now)
                        .bind(agent_id)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to update coach timestamp: {e}"))
                        })?;

                    return self
                        .get_listing(agent_id)
                        .await?
                        .ok_or_else(|| AppError::internal("Failed to fetch updated listing"));
                }

                sqlx::query(INSERT_PENDING_LISTING_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .bind(PublishStatus::PendingReview.as_str())
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create store listing: {e}"))
                    })?;

                sqlx::query(TOUCH_AGENT_SQL)
                    .bind(now)
                    .bind(agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update coach timestamp: {e}"))
                    })?;

                self.get_listing(agent_id)
                    .await?
                    .ok_or_else(|| AppError::internal("Failed to fetch created listing"))
            }

            async fn get_listing(&self, agent_id: &str) -> AppResult<Option<StoreListing>> {
                let row = sqlx::query(GET_LISTING_SQL)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get store listing: {e}")))?;
                row.map(|r| store_listing_from_row(&r)).transpose()
            }

            async fn approve_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
                admin_user_id: Option<Uuid>,
            ) -> AppResult<AgentWithListing> {
                let now = Utc::now();

                let mut tx =
                    self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("Failed to begin approval: {e}"))
                    })?;
                let result = sqlx::query(APPROVE_LISTING_SQL)
                    .bind(PublishStatus::Published.as_str())
                    .bind(now)
                    .bind(admin_user_id.map(|id| id.to_string()))
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to approve coach: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::invalid_input(
                        "Agent not found or not pending review",
                    ));
                }

                ensure_catalogue_handle!(tx, agent_id, tenant_id)?;
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("Failed to commit approval: {e}")))?;

                self.store_agent_with_listing(agent_id, &tenant_id).await
            }

            async fn reject_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
                admin_user_id: Option<Uuid>,
                reason: &str,
            ) -> AppResult<AgentWithListing> {
                let result = sqlx::query(REJECT_LISTING_SQL)
                    .bind(PublishStatus::Rejected.as_str())
                    .bind(Utc::now())
                    .bind(admin_user_id.map(|id| id.to_string()))
                    .bind(reason)
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to reject coach: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::invalid_input(
                        "Agent not found or not pending review",
                    ));
                }

                self.store_agent_with_listing(agent_id, &tenant_id).await
            }

            async fn unpublish_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<AgentWithListing> {
                let result = sqlx::query(UNPUBLISH_LISTING_SQL)
                    .bind(PublishStatus::Draft.as_str())
                    .bind(Utc::now())
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to unpublish coach: {e}")))?;

                if result.rows_affected() == 0 {
                    return Err(AppError::invalid_input("Agent not found or not published"));
                }

                self.store_agent_with_listing(agent_id, &tenant_id).await
            }

            async fn get_pending_review_agents(
                &self,
                tenant_id: TenantId,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<AgentWithListing>> {
                let rows = sqlx::query(PENDING_REVIEW_SQL)
                    .bind(tenant_id.to_string())
                    .bind(page_limit(limit, 50))
                    .bind(page_offset(offset))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get pending review coaches: {e}"))
                    })?;
                rows.iter()
                    .map(|row| agent_with_listing_from_row(row, $agent_row))
                    .collect()
            }

            async fn get_rejected_agents(
                &self,
                tenant_id: TenantId,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<AgentWithListing>> {
                let rows = sqlx::query(REJECTED_SQL)
                    .bind(tenant_id.to_string())
                    .bind(page_limit(limit, 50))
                    .bind(page_offset(offset))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get rejected coaches: {e}"))
                    })?;
                rows.iter()
                    .map(|row| agent_with_listing_from_row(row, $agent_row))
                    .collect()
            }

            async fn get_store_admin_stats(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<StoreAdminStats> {
                let row = sqlx::query(STORE_ADMIN_STATS_SQL)
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get store stats: {e}")))?;
                store_admin_stats_from_row(&row)
            }

            async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>> {
                let row = sqlx::query(AUTHOR_EMAIL_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get author email: {e}")))?;
                row.map(|r| column(&r, "email")).transpose()
            }

            async fn get_published_agents(
                &self,
                category: Option<AgentCategory>,
                sort_by: Option<&str>,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<AgentWithListing>> {
                let order_clause = match sort_by {
                    Some("popular") => "sl.install_count DESC, sl.published_at DESC",
                    Some("title") => "c.title ASC",
                    _ => "sl.published_at DESC",
                };
                let rows = sqlx::query(&published_agents_sql(
                    &category_filter(category),
                    order_clause,
                ))
                .bind(page_limit(limit, 50))
                .bind(page_offset(offset))
                .fetch_all(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to get published coaches: {e}")))?;
                rows.iter()
                    .map(|row| agent_with_listing_from_row(row, $agent_row))
                    .collect()
            }

            async fn get_published_agents_cursor(
                &self,
                category: Option<AgentCategory>,
                sort_by: StoreSortOrder,
                limit: u32,
                cursor: Option<&str>,
            ) -> AppResult<CursorPage<AgentWithListing>> {
                let limit_val = limit.min(100);
                let fetch_limit = i64::from(limit_val) + 1;

                let decoded_cursor = cursor
                    .map(|cursor_str| {
                        StoreCursor::decode(&Cursor::from_string(cursor_str.to_owned()), sort_by)
                            .ok_or_else(|| {
                                AppError::invalid_input("Invalid cursor for current sort order")
                            })
                    })
                    .transpose()?;

                let all_items = self
                    .store_published_page(
                        sort_by,
                        &category_filter(category),
                        decoded_cursor.as_ref(),
                        fetch_limit,
                    )
                    .await?;

                let page_len = limit_val as usize;
                let has_more = all_items.len() > page_len;
                let items: Vec<AgentWithListing> = all_items.into_iter().take(page_len).collect();

                let next_cursor = if has_more {
                    items.last().map(|cwl| {
                        let store_cursor = match sort_by {
                            StoreSortOrder::Newest => StoreCursor::newest(
                                cwl.agent.id.to_string(),
                                cwl.listing.published_at,
                            ),
                            StoreSortOrder::Popular => StoreCursor::popular(
                                cwl.agent.id.to_string(),
                                cwl.listing.install_count,
                                cwl.listing.published_at,
                            ),
                            StoreSortOrder::Title => StoreCursor::title(
                                cwl.agent.id.to_string(),
                                cwl.agent.title.clone(),
                            ),
                        };
                        store_cursor.encode()
                    })
                } else {
                    None
                };

                Ok(CursorPage::new(items, next_cursor, None, has_more))
            }

            async fn search_published_agents(
                &self,
                query: &str,
                limit: Option<u32>,
                locale: &str,
            ) -> AppResult<Vec<AgentWithListing>> {
                let rows = sqlx::query(search_published_sql!($like))
                    .bind(contains_pattern(query))
                    .bind(locale)
                    .bind(page_limit(limit, 20))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to search published coaches: {e}"))
                    })?;
                rows.iter()
                    .map(|row| agent_with_listing_from_row(row, $agent_row))
                    .collect()
            }

            async fn get_published_agent(
                &self,
                agent_id: &str,
            ) -> AppResult<Option<AgentWithListing>> {
                let row = sqlx::query(PUBLISHED_AGENT_SQL)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get published coach: {e}"))
                    })?;
                row.map(|r| agent_with_listing_from_row(&r, $agent_row))
                    .transpose()
            }

            async fn find_published_by_handle(
                &self,
                handle: &AgentHandle,
            ) -> AppResult<Option<AgentWithListing>> {
                let row = sqlx::query(PUBLISHED_BY_HANDLE_SQL)
                    .bind(handle.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to resolve published coach by handle: {e}"
                        ))
                    })?;
                row.map(|r| agent_with_listing_from_row(&r, $agent_row))
                    .transpose()
            }

            async fn get_category_counts(&self) -> AppResult<HashMap<AgentCategory, i64>> {
                let rows = sqlx::query(CATEGORY_COUNTS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get category counts: {e}"))
                    })?;
                let mut counts = HashMap::new();
                for row in &rows {
                    let category: String = column(row, "category")?;
                    let count: i64 = column(row, "count")?;
                    counts.insert(AgentCategory::parse(&category), count);
                }
                Ok(counts)
            }

            async fn increment_install_count(&self, agent_id: &str) -> AppResult<()> {
                sqlx::query(INCREMENT_INSTALLS_SQL)
                    .bind(Utc::now())
                    .bind(agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to increment install count: {e}"))
                    })?;
                Ok(())
            }

            async fn decrement_install_count(&self, agent_id: &str) -> AppResult<()> {
                sqlx::query(DECREMENT_INSTALLS_SQL)
                    .bind(Utc::now())
                    .bind(agent_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to decrement install count: {e}"))
                    })?;
                Ok(())
            }

            async fn install_from_store(
                &self,
                source_agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Agent> {
                let source = self
                    .get_published_agent(source_agent_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::not_found(format!("Published coach {source_agent_id}"))
                    })?;

                let existing = sqlx::query(EXISTING_INSTALL_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(source_agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to check existing installation: {e}"))
                    })?;

                if existing.is_some() {
                    return Err(AppError::invalid_input(format!(
                        "Coach {} is already installed",
                        source.agent.title
                    )));
                }

                // The copy is a personal agent: no store fields, private, not system.
                let now = Utc::now();
                let id = Uuid::new_v4().to_string();
                let tags_json = serde_json::to_string(&source.agent.tags)?;
                let sample_prompts_json = serde_json::to_string(&source.agent.sample_prompts)?;
                let prerequisites_json = serde_json::to_string(&source.agent.prerequisites)?;

                sqlx::query(INSTALL_AGENT_SQL)
                    .bind(&id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(&source.agent.title)
                    .bind(&source.agent.description)
                    .bind(&source.agent.system_prompt)
                    .bind(source.agent.category.as_str())
                    .bind(&tags_json)
                    .bind(&sample_prompts_json)
                    .bind(token_count_column(source.agent.token_count)?)
                    .bind(now)
                    .bind(AgentVisibility::Private.as_str())
                    .bind(&prerequisites_json)
                    .bind(source_agent_id)
                    .bind(&source.agent.handle)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to install coach: {e}")))?;

                sqlx::query(SELF_ASSIGN_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(&id)
                    .bind($ids::bind(user_id))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create coach assignment: {e}"))
                    })?;

                self.increment_install_count(source_agent_id).await?;

                let row = sqlx::query(INSTALLED_AGENT_SQL)
                    .bind(&id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch installed coach: {e}"))
                    })?;
                $agent_row(&row)
            }

            async fn uninstall_agent(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<String> {
                let row = sqlx::query(OWNED_AGENT_ORIGIN_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?
                    .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;

                let source_id: Option<String> = column(&row, "forked_from")?;
                let source_id = source_id.ok_or_else(|| {
                    AppError::invalid_input("This agent was not installed from the Store")
                })?;

                sqlx::query(DELETE_OWNED_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to uninstall coach: {e}")))?;

                self.decrement_install_count(&source_id).await?;

                Ok(source_id)
            }

            async fn get_installed_agents(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<Agent>> {
                let rows = sqlx::query(INSTALLED_AGENTS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get installed coaches: {e}"))
                    })?;
                rows.iter().map($agent_row).collect()
            }

            async fn ensure_listing(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<StoreListing> {
                if let Some(listing) = self.get_listing(agent_id).await? {
                    return Ok(listing);
                }

                sqlx::query(INSERT_DRAFT_LISTING_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(agent_id)
                    .bind(tenant_id.to_string())
                    .bind(PublishStatus::Draft.as_str())
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create store listing: {e}"))
                    })?;

                self.get_listing(agent_id)
                    .await?
                    .ok_or_else(|| AppError::internal("Failed to fetch created listing"))
            }
        }
    };
}
pub(crate) use impl_store_listings_repository;
