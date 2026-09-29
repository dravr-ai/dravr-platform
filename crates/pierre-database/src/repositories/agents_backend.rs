// ABOUTME: The one AgentsRepository implementation, emitted per backend by macro
// ABOUTME: The trait body over the statements in repositories/agents.rs; each backend shell supplies its row type and uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agents, written once.
//!
//! The statements live in [`super::agents`] and the row decoders in
//! [`super::agent_rows`]; this module holds the body that runs them. Each
//! backend's shell invokes [`impl_agents_repository`] with its own type, its
//! driver's row type and its [`super::uuid_columns`] codec, which is the only thing the backends cannot
//! share: `agents.user_id` and every `users(id)` reference are `uuid` on
//! Postgres and TEXT on `SQLite`.

/// Emit the whole `AgentsRepository` implementation for one backend type.
///
/// `$row` is the driver's row type and `$ids` the backend's uuid codec
/// (`SqliteRow` + `TextUuid`, `PgRow` + `NativeUuid`). The body names its
/// consts, helpers, decoders and types unqualified, so the invoking shell
/// must `use` every one of them.
macro_rules! impl_agents_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// Decode an `agent_assignments` row joined to the user's email.
        fn row_to_assignment(row: &$row) -> AppResult<AgentAssignment> {
            Ok(AgentAssignment {
                user_id: $ids::read_text(row, "user_id")?,
                user_email: column(row, "email")?,
                assigned_at: instant(row, "created_at")?.to_rfc3339(),
                assigned_by: $ids::read_text_opt(row, "assigned_by")?,
            })
        }

        impl $ty {
            /// Make sure the user holds an assignment row for the agent.
            async fn ensure_agent_assignment(
                &self,
                agent_id: &str,
                user_id: Uuid,
            ) -> AppResult<()> {
                sqlx::query(ENSURE_ASSIGNMENT_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to ensure coach assignment: {e}"))
                    })?;
                Ok(())
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
                        },
                    );
                }
                Ok(overlays)
            }
        }

        #[async_trait::async_trait]
        impl AgentsRepository for $ty {
            async fn create(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                request: &CreateAgentRequest,
            ) -> AppResult<Agent> {
                let now = Utc::now();
                let id = Uuid::new_v4();
                // Structured `instructions`, when given, are the runtime prompt.
                let effective_system_prompt = request
                    .instructions
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&request.system_prompt);
                let agent = Agent {
                    id,
                    user_id,
                    tenant_id: tenant_id.to_string(),
                    title: request.title.clone(),
                    description: request.description.clone(),
                    system_prompt: effective_system_prompt.to_owned(),
                    category: request.category,
                    tags: request.tags.clone(),
                    sample_prompts: request.sample_prompts.clone(),
                    token_count: 0,
                    created_at: now,
                    updated_at: now,
                    is_system: false,
                    visibility: AgentVisibility::Private,
                    prerequisites: AgentPrerequisites::default(),
                    forked_from: None,
                    handle: None,
                    max_tool_iterations: request.max_tool_iterations,
                    temperature: None,
                    startup_query: request.startup_query.clone(),
                    data_requirements: request.data_requirements.clone(),
                    purpose: request.purpose.clone(),
                    when_to_use: request.when_to_use.clone(),
                    instructions: request.instructions.clone(),
                    example_inputs: request.example_inputs.clone(),
                    example_outputs: request.example_outputs.clone(),
                    success_criteria: request.success_criteria.clone(),
                    source: "custom".to_owned(),
                };
                // The section-aware count the model defines, one rule for both backends.
                let token_count = agent.compute_token_count();
                let data_requirements_json = request
                    .data_requirements
                    .as_ref()
                    .and_then(|dr| serde_json::to_string(dr).ok());

                sqlx::query(INSERT_AGENT_SQL)
                    .bind(id.to_string())
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(&request.title)
                    .bind(&request.description)
                    .bind(effective_system_prompt)
                    .bind(request.category.as_str())
                    .bind(serde_json::to_string(&request.tags)?)
                    .bind(serde_json::to_string(&request.sample_prompts)?)
                    .bind(token_count_bind(token_count))
                    .bind(now)
                    .bind(AgentVisibility::Private.as_str())
                    .bind(request.max_tool_iterations)
                    .bind(&request.startup_query)
                    .bind(&data_requirements_json)
                    .bind(&request.purpose)
                    .bind(&request.when_to_use)
                    .bind(&request.instructions)
                    .bind(&request.example_inputs)
                    .bind(&request.example_outputs)
                    .bind(&request.success_criteria)
                    .bind(compute_request_hash(request))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create coach: {e}")))?;

                // The creator's own assignment row.
                self.ensure_agent_assignment(&id.to_string(), user_id)
                    .await?;

                Ok(Agent {
                    token_count,
                    ..agent
                })
            }

            async fn get_by_id(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                self.fetch_one_agent(
                    sqlx::query(GET_AGENT_SQL)
                        .bind(agent_id)
                        .bind($ids::bind(user_id))
                        .bind(tenant_id),
                    "get coach",
                )
                .await
            }

            async fn get_in_tenant(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                self.fetch_one_agent(
                    sqlx::query(GET_AGENT_IN_TENANT_SQL)
                        .bind(agent_id)
                        .bind(tenant_id),
                    "get tenant coach",
                )
                .await
            }

            async fn find_installed_by_handle(
                &self,
                handle: &AgentHandle,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                self.fetch_one_agent(
                    sqlx::query(FIND_INSTALLED_BY_HANDLE_SQL)
                        .bind($ids::bind(user_id))
                        .bind(handle.as_str())
                        .bind(tenant_id),
                    "resolve coach by handle",
                )
                .await
            }

            async fn list(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                filter: &ListAgentsFilter,
            ) -> AppResult<Vec<AgentListItem>> {
                let limit = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
                let offset = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);
                let rows = sqlx::query(LIST_AGENTS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(limit)
                    .bind(offset)
                    .bind(filter.include_system)
                    .bind(filter.category.as_ref().map(AgentCategory::as_str))
                    .bind(filter.favorites_only)
                    .bind(filter.include_hidden)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list coaches: {e}")))?;
                rows.iter().map(row_to_agent_list_item).collect()
            }

            async fn apply_translations(
                &self,
                agents: &mut [AgentListItem],
                locale: &str,
            ) -> AppResult<()> {
                let ids: Vec<String> = agents.iter().map(|it| it.agent.id.to_string()).collect();
                let overlays = self.agent_translation_overlays(&ids, locale).await?;
                for item in agents.iter_mut() {
                    if let Some(ov) = overlays.get(&item.agent.id.to_string()) {
                        ov.apply(&mut item.agent);
                    }
                }
                Ok(())
            }

            async fn translate_agents(&self, agents: &mut [Agent], locale: &str) -> AppResult<()> {
                let ids: Vec<String> = agents.iter().map(|c| c.id.to_string()).collect();
                let overlays = self.agent_translation_overlays(&ids, locale).await?;
                for agent in agents.iter_mut() {
                    if let Some(ov) = overlays.get(&agent.id.to_string()) {
                        ov.apply(agent);
                    }
                }
                Ok(())
            }

            async fn update(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
                request: &UpdateAgentRequest,
                change_summary: Option<&str>,
            ) -> AppResult<Option<Agent>> {
                let Some(existing) =
                    AgentsRepository::get_by_id(self, agent_id, user_id, tenant_id).await?
                else {
                    return Ok(None);
                };
                // Snapshot before the change, carrying the caller's summary.
                self.create_version(agent_id, user_id, change_summary)
                    .await?;

                let title = request.title.as_ref().unwrap_or(&existing.title);
                let description = request.description.clone().or(existing.description);
                let system_prompt = request
                    .system_prompt
                    .as_ref()
                    .unwrap_or(&existing.system_prompt);
                let category = request.category.unwrap_or(existing.category);
                let tags = request.tags.as_ref().unwrap_or(&existing.tags);
                let sample_prompts = request
                    .sample_prompts
                    .as_ref()
                    .unwrap_or(&existing.sample_prompts);
                let token_count = estimate_prompt_tokens(system_prompt);
                let startup_query: Option<String> = if request.startup_query.is_some() {
                    request
                        .startup_query
                        .as_ref()
                        .filter(|q| !q.is_empty())
                        .cloned()
                } else {
                    self.stored_agent_text(AGENT_STARTUP_QUERY_SQL, agent_id, "startup_query")
                        .await?
                };
                let data_requirements_json: Option<String> = if request.data_requirements.is_some()
                {
                    request
                        .data_requirements
                        .as_ref()
                        .and_then(|dr| serde_json::to_string(dr).ok())
                } else {
                    self.stored_agent_text(
                        AGENT_DATA_REQUIREMENTS_SQL,
                        agent_id,
                        "data_requirements",
                    )
                    .await?
                };
                let purpose = request.purpose.clone().or(existing.purpose);
                let when_to_use = request.when_to_use.clone().or(existing.when_to_use);
                let instructions = request.instructions.clone().or(existing.instructions);
                let example_inputs = request.example_inputs.clone().or(existing.example_inputs);
                let example_outputs = request.example_outputs.clone().or(existing.example_outputs);
                let success_criteria = request
                    .success_criteria
                    .clone()
                    .or(existing.success_criteria);
                // Three-way, not a coalesce: an absent field keeps the stored
                // budget, an explicit null clears it back to the admin value.
                let max_tool_iterations = request
                    .max_tool_iterations
                    .resolve(existing.max_tool_iterations);
                // Updated instructions are the runtime prompt too.
                let system_prompt = instructions
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(system_prompt);

                let result = sqlx::query(UPDATE_AGENT_SQL)
                    .bind(title)
                    .bind(&description)
                    .bind(system_prompt)
                    .bind(category.as_str())
                    .bind(serde_json::to_string(tags)?)
                    .bind(serde_json::to_string(sample_prompts)?)
                    .bind(token_count_bind(token_count))
                    .bind(Utc::now())
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(&startup_query)
                    .bind(&data_requirements_json)
                    .bind(&purpose)
                    .bind(&when_to_use)
                    .bind(&instructions)
                    .bind(&example_inputs)
                    .bind(&example_outputs)
                    .bind(&success_criteria)
                    .bind(max_tool_iterations)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                AgentsRepository::get_by_id(self, agent_id, user_id, tenant_id).await
            }

            async fn delete(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to delete coach: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn record_usage(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                if !self.agent_reachable(agent_id, tenant_id).await? {
                    return Ok(false);
                }
                self.ensure_agent_assignment(agent_id, user_id).await?;
                let result = sqlx::query(RECORD_USAGE_SQL)
                    .bind(Utc::now())
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record coach usage: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn toggle_favorite(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<bool>> {
                if !self.agent_reachable(agent_id, tenant_id).await? {
                    return Ok(None);
                }
                self.ensure_agent_assignment(agent_id, user_id).await?;
                let row = sqlx::query(FAVORITE_STATUS_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get favorite status: {e}"))
                    })?;
                let current: bool = match row {
                    Some(row) => column(&row, "is_favorite")?,
                    None => false,
                };
                let new_value = !current;
                sqlx::query(SET_FAVORITE_SQL)
                    .bind(new_value)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to toggle favorite: {e}")))?;
                Ok(Some(new_value))
            }

            async fn search(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                query: &str,
                category: Option<AgentCategory>,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<Agent>> {
                let rows = sqlx::query(SEARCH_AGENTS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .bind(format!("%{query}%"))
                    .bind(i32::try_from(limit.unwrap_or(20)).unwrap_or(20))
                    .bind(i32::try_from(offset.unwrap_or(0)).unwrap_or(0))
                    .bind(category.as_ref().map(AgentCategory::as_str))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to search coaches: {e}")))?;
                rows.iter().map(row_to_agent).collect()
            }

            async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
                let row = sqlx::query(COUNT_AGENTS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count coaches: {e}")))?;
                let count: i64 = column(&row, "count")?;
                Ok(u32::try_from(count).unwrap_or(u32::MAX))
            }

            async fn activate_agent(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                if !self.agent_reachable(agent_id, tenant_id).await? {
                    return Ok(None);
                }
                // The roster row still records entitlement, favourites and usage.
                self.ensure_agent_assignment(agent_id, user_id).await?;
                sqlx::query(SELECT_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to activate coach: {e}")))?;
                AgentsRepository::get_by_id(self, agent_id, user_id, tenant_id).await
            }

            async fn deactivate_agent(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DESELECT_AGENT_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to deactivate coach: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn get_active_agent(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                self.fetch_one_agent(
                    sqlx::query(GET_ACTIVE_AGENT_SQL)
                        .bind($ids::bind(user_id))
                        .bind(tenant_id),
                    "get active coach",
                )
                .await
            }

            async fn find_by_content_hash(
                &self,
                content_hash: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                self.fetch_one_agent(
                    sqlx::query(FIND_BY_CONTENT_HASH_SQL)
                        .bind(content_hash)
                        .bind($ids::bind(user_id))
                        .bind(tenant_id),
                    "find coach by content hash",
                )
                .await
            }

            async fn create_system_agent(
                &self,
                admin_user_id: Uuid,
                tenant_id: TenantId,
                request: &CreateSystemAgentRequest,
            ) -> AppResult<Agent> {
                let now = Utc::now();
                let id = Uuid::new_v4();
                let token_count = estimate_prompt_tokens(&request.system_prompt);
                sqlx::query(INSERT_SYSTEM_AGENT_SQL)
                    .bind(id.to_string())
                    .bind($ids::bind(admin_user_id))
                    .bind(tenant_id)
                    .bind(&request.title)
                    .bind(&request.description)
                    .bind(&request.system_prompt)
                    .bind(request.category.as_str())
                    .bind(serde_json::to_string(&request.tags)?)
                    .bind(serde_json::to_string(&request.sample_prompts)?)
                    .bind(token_count_bind(token_count))
                    .bind(now)
                    .bind(request.visibility.as_str())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create system agent: {e}"))
                    })?;

                Ok(Agent {
                    id,
                    user_id: admin_user_id,
                    tenant_id: tenant_id.to_string(),
                    title: request.title.clone(),
                    description: request.description.clone(),
                    system_prompt: request.system_prompt.clone(),
                    category: request.category,
                    tags: request.tags.clone(),
                    sample_prompts: request.sample_prompts.clone(),
                    token_count,
                    created_at: now,
                    updated_at: now,
                    is_system: true,
                    visibility: request.visibility,
                    prerequisites: AgentPrerequisites::default(),
                    forked_from: None,
                    handle: None,
                    max_tool_iterations: None,
                    temperature: None,
                    startup_query: None,
                    data_requirements: None,
                    purpose: None,
                    when_to_use: None,
                    instructions: None,
                    example_inputs: None,
                    example_outputs: None,
                    success_criteria: None,
                    source: "custom".to_owned(),
                })
            }

            async fn list_system_agents(&self, tenant_id: TenantId) -> AppResult<Vec<Agent>> {
                let rows = sqlx::query(LIST_SYSTEM_AGENTS_SQL)
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list system agents: {e}"))
                    })?;
                rows.iter().map(row_to_agent).collect()
            }

            async fn get_system_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<Agent>> {
                self.fetch_one_agent(
                    sqlx::query(GET_SYSTEM_AGENT_SQL)
                        .bind(agent_id)
                        .bind(tenant_id),
                    "get system agent",
                )
                .await
            }

            async fn update_system_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
                request: &UpdateAgentRequest,
            ) -> AppResult<Option<Agent>> {
                let Some(existing) = self.get_system_agent(agent_id, tenant_id).await? else {
                    return Ok(None);
                };
                // The version is recorded against the admin who created the agent.
                self.create_version(agent_id, existing.user_id, None)
                    .await?;
                let title = request.title.as_ref().unwrap_or(&existing.title);
                let description = request.description.clone().or(existing.description);
                let system_prompt = request
                    .system_prompt
                    .as_ref()
                    .unwrap_or(&existing.system_prompt);
                let category = request.category.unwrap_or(existing.category);
                let tags = request.tags.as_ref().unwrap_or(&existing.tags);
                let sample_prompts = request
                    .sample_prompts
                    .as_ref()
                    .unwrap_or(&existing.sample_prompts);
                let result = sqlx::query(UPDATE_SYSTEM_AGENT_SQL)
                    .bind(title)
                    .bind(&description)
                    .bind(system_prompt)
                    .bind(category.as_str())
                    .bind(serde_json::to_string(tags)?)
                    .bind(serde_json::to_string(sample_prompts)?)
                    .bind(token_count_bind(estimate_prompt_tokens(system_prompt)))
                    .bind(Utc::now())
                    .bind(agent_id)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update system agent: {e}"))
                    })?;
                if result.rows_affected() == 0 {
                    return Ok(None);
                }
                self.get_system_agent(agent_id, tenant_id).await
            }

            async fn delete_system_agent(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_SYSTEM_AGENT_SQL)
                    .bind(agent_id)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete system agent: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn get_user_preferences(
                &self,
                agent_id: &str,
                user_id: Uuid,
            ) -> AppResult<(bool, u32, Option<DateTime<Utc>>)> {
                let row = sqlx::query(USER_PREFERENCES_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get user preferences: {e}"))
                    })?;
                let Some(row) = row else {
                    return Ok((false, 0, None));
                };
                let use_count: i32 = column(&row, "use_count")?;
                Ok((
                    column(&row, "is_favorite")?,
                    u32::try_from(use_count).unwrap_or(0),
                    column(&row, "last_used_at")?,
                ))
            }

            async fn assign_agent(
                &self,
                agent_id: &str,
                user_id: Uuid,
                assigned_by: Uuid,
            ) -> AppResult<bool> {
                let result = sqlx::query(ASSIGN_AGENT_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(assigned_by))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn unassign_agent(&self, agent_id: &str, user_id: Uuid) -> AppResult<bool> {
                let result = sqlx::query(UNASSIGN_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn list_assignments(&self, agent_id: &str) -> AppResult<Vec<AgentAssignment>> {
                let rows = sqlx::query(LIST_ASSIGNMENTS_SQL)
                    .bind(agent_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
                rows.iter().map(row_to_assignment).collect()
            }

            async fn list_assignments_for_tenant(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<AgentAssignment>> {
                let rows = sqlx::query(LIST_ASSIGNMENTS_FOR_TENANT_SQL)
                    .bind(agent_id)
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
                rows.iter().map(row_to_assignment).collect()
            }

            async fn hide_agent(
                &self,
                agent_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                if !self.agent_hideable(agent_id, user_id, tenant_id).await? {
                    return Err(AppError::invalid_input(
                        "Only system or assigned coaches can be hidden",
                    ));
                }
                sqlx::query(HIDE_AGENT_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind($ids::bind(user_id))
                    .bind(agent_id)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;
                Ok(true)
            }

            async fn show_agent(&self, agent_id: &str, user_id: Uuid) -> AppResult<bool> {
                let result = sqlx::query(SHOW_AGENT_SQL)
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn list_hidden_agents(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<Agent>> {
                let rows = sqlx::query(LIST_HIDDEN_AGENTS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list hidden coaches: {e}"))
                    })?;
                rows.iter().map(row_to_agent).collect()
            }

            async fn create_version(
                &self,
                agent_id: &str,
                user_id: Uuid,
                change_summary: Option<&str>,
            ) -> AppResult<i32> {
                let agent = self
                    .fetch_one_agent(
                        sqlx::query(AGENT_BY_ID_SQL).bind(agent_id),
                        "get coach for versioning",
                    )
                    .await?
                    .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;
                let new_version = self.get_current_version(agent_id).await? + 1;
                let content_snapshot = agent.content_snapshot();
                sqlx::query(INSERT_VERSION_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(agent_id)
                    .bind(new_version)
                    .bind(compute_content_hash(&content_snapshot))
                    .bind(content_snapshot.to_string())
                    .bind(change_summary)
                    .bind(Utc::now())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create version: {e}")))?;
                Ok(new_version)
            }

            async fn get_versions(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
                limit: u32,
            ) -> AppResult<Vec<AgentVersion>> {
                self.require_agent_in_tenant(agent_id, tenant_id).await?;
                let rows = sqlx::query(LIST_VERSIONS_SQL)
                    .bind(agent_id)
                    .bind(i32::try_from(limit).unwrap_or(50))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get versions: {e}")))?;
                rows.iter().map(row_to_agent_version).collect()
            }

            async fn get_version(
                &self,
                agent_id: &str,
                version: i32,
                tenant_id: TenantId,
            ) -> AppResult<Option<AgentVersion>> {
                self.require_agent_in_tenant(agent_id, tenant_id).await?;
                let row = sqlx::query(GET_VERSION_SQL)
                    .bind(agent_id)
                    .bind(version)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get version: {e}")))?;
                row.as_ref().map(row_to_agent_version).transpose()
            }

            async fn revert_to_version(
                &self,
                agent_id: &str,
                version: i32,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Agent> {
                let target = self
                    .get_version(agent_id, version, tenant_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::not_found(format!("Version {version} for coach {agent_id}"))
                    })?;
                // Only the owner may revert; refuse before anything is written,
                // so a denied call leaves neither the agent nor its history
                // touched.
                let owned = self
                    .fetch_one_agent(
                        sqlx::query(AGENT_OF_TENANT_SQL)
                            .bind(agent_id)
                            .bind(tenant_id),
                        "get coach for revert",
                    )
                    .await?
                    .is_some_and(|agent| agent.user_id == user_id);
                if !owned {
                    return Err(AppError::not_found(format!("Coach {agent_id}")));
                }
                let snapshot = &target.content_snapshot;
                let title = snapshot
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| AppError::internal("Missing title in version snapshot"))?;
                let description = snapshot
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let system_prompt = snapshot
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AppError::internal("Missing system_prompt in version snapshot")
                    })?;
                let category = snapshot
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("custom");
                let tags: Vec<String> = snapshot
                    .get("tags")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let sample_prompts: Vec<String> = snapshot
                    .get("sample_prompts")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                // A revert is an edit: snapshot the content it replaces first,
                // exactly as `update` does, so the latest edit stays in history
                // and the revert itself can be undone.
                let summary = format!("Reverted to version {version}");
                self.create_version(agent_id, user_id, Some(&summary))
                    .await?;

                let result = sqlx::query(REVERT_AGENT_SQL)
                    .bind(title)
                    .bind(&description)
                    .bind(system_prompt)
                    .bind(category)
                    .bind(serde_json::to_string(&tags)?)
                    .bind(serde_json::to_string(&sample_prompts)?)
                    .bind(token_count_bind(estimate_prompt_tokens(system_prompt)))
                    .bind(Utc::now())
                    .bind(agent_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to revert coach: {e}")))?;
                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!("Coach {agent_id}")));
                }

                self.fetch_one_agent(
                    sqlx::query(AGENT_OF_TENANT_SQL)
                        .bind(agent_id)
                        .bind(tenant_id),
                    "get reverted coach",
                )
                .await?
                .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))
            }

            async fn get_current_version(&self, agent_id: &str) -> AppResult<i32> {
                let row = sqlx::query(MAX_VERSION_SQL)
                    .bind(agent_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get current version: {e}"))
                    })?;
                column(&row, "max_version")
            }

            async fn get_agent_runtime_context(
                &self,
                agent_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<AgentRuntimeContext>> {
                let row = sqlx::query(RUNTIME_CONTEXT_SQL)
                    .bind(agent_id)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get coach runtime context: {e}"))
                    })?;
                let Some(row) = row else {
                    return Ok(None);
                };
                let slug: Option<String> = column(&row, "slug")?;
                let visuals: Option<String> = column(&row, "visuals")?;
                let category: String = column(&row, "category")?;
                // DOUBLE PRECISION on Postgres, REAL on SQLite: read wide.
                let temperature: Option<f64> = column(&row, "temperature")?;
                #[allow(clippy::cast_possible_truncation)]
                Ok(Some(AgentRuntimeContext {
                    slug: slug.unwrap_or_default(),
                    title: column(&row, "title")?,
                    source: column(&row, "source")?,
                    system_prompt: column(&row, "system_prompt")?,
                    startup_query: column(&row, "startup_query")?,
                    data_requirements: column(&row, "data_requirements")?,
                    visuals: split_visuals(visuals.as_deref()),
                    max_tool_iterations: column(&row, "max_tool_iterations")?,
                    temperature: temperature.map(|t| t as f32),
                    category: AgentCategory::parse(&category),
                }))
            }
        }
    };
}
pub(crate) use impl_agents_repository;
