// ABOUTME: The athlete profile tool, read from a provider, and the athlete's totals, a view over the one aggregator
// ABOUTME: get_stats answers all-time and this-year totals over every source merged, never one provider's figure

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete Profile and Statistics Tools
//!
//! - `GetAthleteTool` — the athlete profile a connected provider reports,
//!   read live within the refresh budget, else the last-known copy.
//! - `GetStatsTool` — the athlete's all-time and this-year totals: one read
//!   of the store through the aggregator ([`query_activities`]), every
//!   source merged, so it agrees with `query_activities` by construction
//!   (carnet#1485).

use crate::annotations::{annotations, Effect, Reach};
use crate::protocol::provider_helpers::PROVIDER_ARGUMENT;
use crate::read_subject::DataClass;
use crate::tool_data::ToolData;
use dravr_barrage::Stream;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pierre_core::errors::protocol::ProtocolError;
use pierre_core::models::coverage::SourceReach;
use pierre_core::models::provider_context::ProviderAthleteContext;
use pierre_core::models::{Athlete, PeriodTotals, Stats};
use pierre_core::models::{GearKind, TenantId};
use pierre_database::repositories::PersonalBest;
use pierre_database::RepositoryRegistry;
use pierre_formatters::OutputFormat;
use pierre_providers::core::FitnessProvider;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use pierre_cache::{Cache, CacheKey, CacheResource};
use uuid::Uuid;

use crate::activity_query::{
    parse_query, query_activities, Athlete as QueriedAthlete, GroupKey, KeyValue, MetricValues,
    QueryAnswer, ReadGate,
};
use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, format_property, object_schema, task_capable, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::activity_query::athlete_today;
use crate::implementations::data_helpers::parse_output_format;
use crate::implementations::handler_bridge;
use crate::implementations::last_known::{self, Fallback};
use crate::implementations::provider_context_reads::{
    read_athlete_context, source_label, ContextLimits, LedgerShoes, CONTEXT_READ_BUDGET,
};
use crate::protocol::format::formatted_response;
use crate::protocol::provider_helpers::resolve_provider_for_tool;
use crate::protocol::types::UniversalResponse;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver;
use pierre_providers::provider_context::serves_context;
use pierre_providers::registry::ProviderRegistry;
use pierre_tools_core::ToolResult;
use tokio::time;

/// What `get_athlete` answers with.
///
/// The provider's athlete under an `athlete` key rather than at the top
/// level. The key is the contract: `GetAthleteResponseSchema` in the
/// TypeScript SDK reads `athlete`, and `provider_backend_resolver_test`
/// asserts it. Unwrapping it to save one level would break both.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetAthleteResult {
    /// The athlete profile as the provider reports it.
    pub athlete: Athlete,
    /// The provider's own figures about the athlete, read live on this call
    /// and labelled with the provider they come from: body weight, all-time
    /// records, the bikes it keeps and, while the athlete declared no shoes in
    /// Dravr, its shoes, with their mileage. Context, never Dravr's: the
    /// weight is not the athlete's weight anchor, the records sit beside
    /// `dravr_personal_bests` without being reconciled with them, and the gear
    /// is not in the gear ledger. Quote them as the provider's ("Strava has
    /// your 5K at 20:12"). Absent when the provider holds none, serves none,
    /// or did not answer in time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_context: Option<ProviderAthleteContext>,
    /// The bests Dravr computed from the athlete's own runs, labelled as
    /// Dravr's. Beside the provider's records, never reconciled with them: the
    /// two are measured differently and may disagree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dravr_personal_bests: Option<DravrPersonalBests>,
}

/// Dravr's own computed personal bests, labelled with their source.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct DravrPersonalBests {
    /// Whose bests these are: always Dravr's computation.
    pub source: String,
    /// How to quote the block.
    pub note: String,
    /// The fastest effort at each standard distance.
    pub bests: Vec<DravrPersonalBest>,
}

/// One of Dravr's computed bests.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct DravrPersonalBest {
    /// The distance's catalogue code: `5k`, `10k`, `half_marathon` or `marathon`.
    pub distance: String,
    /// Elapsed time of the fastest window covering the distance, seconds.
    pub elapsed_seconds: f64,
    /// The provider the run came from, as the athlete knows it.
    pub provider: String,
    /// The provider's id for the run that set the best.
    pub activity_id: String,
    /// When that run started, RFC 3339.
    pub achieved_at: String,
}

/// The label of Dravr's own bests beside a provider's records.
const DRAVR_BESTS_SOURCE: &str = "Dravr";

/// How to quote Dravr's bests.
const DRAVR_BESTS_NOTE: &str = "Dravr's own computed bests, from the athlete's runs it has \
     measured. Quote them as Dravr's. They sit beside the provider's records and are not \
     reconciled with them: the two are measured differently and may disagree.";

/// The longest a profile served from the cache waits for the provider's own
/// figures: a slow provider never delays the cached answer past it.
pub(crate) const CACHE_HIT_CONTEXT_WAIT: Duration = Duration::from_secs(2);

/// What `get_stats` answers with.
///
/// Same shape of contract as [`GetAthleteResult`]: the totals live under a
/// `stats` key that the SDK schema reads by name.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetStatsResult {
    /// All-time and this-year totals over every source, merged, and how far
    /// the history behind them reaches.
    pub stats: Stats,
    /// Each source's part of the coverage.
    pub coverage_by_source: Vec<SourceReach>,
}

// ============================================================================
// GetAthleteTool - Get athlete profile
// ============================================================================

/// Tool for retrieving the user's athlete profile from a fitness provider.
#[derive(Debug)]
pub struct GetAthleteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetAthleteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(PROVIDER_ARGUMENT.to_owned()),
                ..Default::default()
            },
        );

        properties.insert("format".to_owned(), format_property());

        let schema = object_schema(properties, None);

        answers_with::<Formatted<GetAthleteResult>>(task_capable(tool_definition(
            "get_athlete",
            "Get the athlete profile a connected provider reports: the provider's athlete ID, \
             username, first and last name where public, and profile picture.",
            schema,
            annotations(
                "Get athlete profile",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // PROFILE: this returns who the athlete IS — name, sex, weight,
        // preferences — not the training they have accumulated, so it is
        // reached under `profile:read` and not `fitness:read`.
        PROVIDER_READ | ToolCapabilities::PROFILE
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let provider_name = match resolve_provider_for_tool(&args, &context).await {
                Ok(p) => p,
                Err(result) => return Ok(result),
            };

            // Canonicalize to the serving backend before any cache op — same
            // single-key rule as get_activities: an explicit "garmin" arg and
            // the stored "sciotte_garmin" connection must hit ONE profile key,
            // not two parallel ones that each miss and re-scrape.
            let provider_name = backend_resolver::resolve_backend(
                context.resources.repos(),
                context.user_id,
                context.tenant_id.map(TenantId::from_uuid),
                &provider_name,
            )
            .await;
            // A profile carries no item provenance: a provider whose terms keep
            // its data off this transport is refused before the cache can
            // replay one a first-party call stored (carnet#724).
            ai_scope::first_party_only_read(
                context.resources.provider_registry().as_ref(),
                &provider_name,
                None,
            )?;

            let output_format = parse_output_format(&args);

            let tenant_id = TenantId::from_uuid(context.tenant_id.unwrap_or_else(Uuid::nil));
            let tenant_id_str = context.tenant_id.map(|t| t.to_string());

            let cache_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::AthleteProfile,
            );

            let cache = context.resources.cache();

            // Dravr's own bests are a local read, labelled as Dravr's beside
            // whatever the provider reports; they never wait on the provider.
            let dravr_personal_bests = read_dravr_bests(
                context.resources.repos(),
                context.resources.provider_registry(),
                context.user_id,
                tenant_id,
            )
            .await;

            // The provider's own figures are read live on every call, never
            // stored. Only a backend whose provider serves them is asked at
            // all (decided from its name, before any I/O); the read runs
            // beside the profile's own read on a miss and is bounded to
            // `CACHE_HIT_CONTEXT_WAIT` on a hit, so it never delays either.
            let executor = UniversalExecutor::new(context.resources.clone());
            let reader = serves_context(&provider_name).then(|| AthleteContextRead {
                repos: Arc::clone(context.resources.repos()),
                registry: Arc::clone(context.resources.provider_registry()),
                user_id: context.user_id,
                backend: provider_name.clone(),
            });

            // Cache hit short-circuits the provider fetch round-trip.
            if let Some(cached) = lookup_cached_athlete(cache, &cache_key).await {
                let provider_context = match &reader {
                    Some(reader) => time::timeout(CACHE_HIT_CONTEXT_WAIT, async {
                        let provider = executor
                            .auth_service
                            .create_authenticated_provider(
                                &provider_name,
                                context.user_id,
                                tenant_id_str.as_deref(),
                            )
                            .await
                            .ok()?;
                        reader.read(provider.as_ref()).await
                    })
                    .await
                    .unwrap_or_else(|_| {
                        warn!("the provider's figures were not read in time for a cached profile");
                        None
                    }),
                    None => None,
                };
                let response = handler_bridge::map_protocol_result(
                    "get_athlete",
                    formatted_response(
                        &GetAthleteResult {
                            athlete: cached,
                            provider_context,
                            dravr_personal_bests,
                        },
                        output_format,
                        cached_metadata(context.user_id, tenant_id_str.as_ref()),
                    ),
                )?;
                // The provider's profile of the athlete, read outside every
                // list filter: the result's trust label names its source.
                ai_scope::note_served_from(&provider_name);
                return handler_bridge::map_universal_response("get_athlete", Ok(response));
            }

            // Cache miss path — fetch from the provider.
            let authenticated = executor
                .auth_service
                .create_authenticated_provider(
                    &provider_name,
                    context.user_id,
                    tenant_id_str.as_deref(),
                )
                .await;
            let provider = match authenticated {
                Ok(provider) => provider,
                Err(response) => {
                    let fallback_error = response
                        .error
                        .clone()
                        .unwrap_or_else(|| "get_athlete authentication failed".to_owned());
                    let error_payload = response.result.unwrap_or_else(|| {
                        json!({
                            "error": fallback_error,
                        })
                    });
                    return Ok(ToolResult::error(error_payload));
                }
            };

            // Read live within the refresh budget; past it, or on a failure,
            // the last-known profile stands in with a caveat.
            let last_known_key = CacheKey::new(
                tenant_id,
                context.user_id,
                provider_name.clone(),
                CacheResource::LastKnownAthleteProfile,
            );
            let read = {
                let (cache, cache_key, last_known_key) =
                    (Arc::clone(cache), cache_key.clone(), last_known_key.clone());
                let (user_id, tenant) = (context.user_id, tenant_id_str.clone());
                let dravr_personal_bests = dravr_personal_bests.clone();
                async move {
                    let keys = [&cache_key, &last_known_key];
                    fetch_and_cache_athlete(
                        provider.as_ref(),
                        &cache,
                        keys,
                        user_id,
                        tenant,
                        output_format,
                        BesideProfile {
                            reader,
                            dravr_personal_bests,
                        },
                    )
                    .await
                    .unwrap_or_else(|e| last_known::failed_read(&e))
                }
            };
            let fallback = Fallback {
                key: &last_known_key,
                backend: &provider_name,
                what: "profile",
            };
            let response = last_known::live_or_last_known(
                cache,
                &context.resources.config().timeouts,
                fallback,
                read,
                |athlete| {
                    let metadata = cached_metadata(context.user_id, tenant_id_str.as_ref());
                    formatted_response(
                        &GetAthleteResult {
                            athlete,
                            provider_context: None,
                            dravr_personal_bests: dravr_personal_bests.clone(),
                        },
                        output_format,
                        metadata,
                    )
                },
            )
            .await?;
            if response.success {
                ai_scope::note_served_from(&provider_name);
            }
            handler_bridge::map_universal_response("get_athlete", Ok(response))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetStatsTool - the athlete's totals, a view over the one aggregator
// ============================================================================

/// Tool answering the athlete's all-time and this-year totals: a view over
/// the one aggregator ([`query_activities`]), every source merged, never a
/// provider's own figure.
#[derive(Debug)]
pub struct GetStatsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetStatsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("One source only.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert("format".to_owned(), format_property());

        let schema = object_schema(properties, None);

        answers_with::<Formatted<GetStatsResult>>(tool_definition(
            "get_stats",
            "All-time totals over every source, merged; year_to_date is this calendar year. \
             Read stats.coverage before quoting. For a period or breakdown, call \
             query_activities.",
            schema,
            annotations(
                "Get activity totals",
                Effect::ReadOnly,
                Reach::AthleteAccount,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let athlete = QueriedAthlete {
                user_id: context.user_id,
                tenant_id: TenantId::from_uuid(context.require_tenant()?),
            };
            let runtime = &context.resources;
            let (today, _) = athlete_today(runtime, athlete).await?;
            let mut query = parse_query(&json!({ "provider": args.get("provider") }), today)?;
            query.group_by = vec![GroupKey::Year];
            let answer = query_activities(
                runtime,
                athlete.user_id,
                athlete.tenant_id,
                &query,
                ReadGate::Model,
            )
            .await?;
            let payload = stats_view(answer, today.year());
            let response = formatted_response(
                &payload,
                parse_output_format(&args),
                HashMap::<_, Value>::new(),
            )?;
            handler_bridge::map_universal_response("get_stats", Ok(response))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// One period's totals from the aggregator's values; zeros where nothing
/// was counted.
fn period_totals(values: &MetricValues) -> PeriodTotals {
    PeriodTotals {
        total_activities: values.count.unwrap_or(0),
        total_distance: values.distance_metres().unwrap_or(0.0),
        total_duration: values.elapsed_time_s.unwrap_or(0),
        total_elevation_gain: values.elevation_m.unwrap_or(0.0),
    }
}

/// `get_stats`' wire from one all-time count grouped by year: the totals
/// over every year, and `year`'s group as the year to date (zeros when that
/// year holds nothing).
fn stats_view(answer: QueryAnswer, year: i32) -> GetStatsResult {
    let all_time = period_totals(&answer.totals);
    let this_year = answer
        .groups
        .iter()
        .find(|group| group.key.get(&GroupKey::Year) == Some(&KeyValue::Year(year)))
        .map_or_else(
            || period_totals(&MetricValues::default()),
            |group| period_totals(&group.values),
        );
    GetStatsResult {
        stats: Stats {
            total_activities: all_time.total_activities,
            total_distance: all_time.total_distance,
            total_duration: all_time.total_duration,
            total_elevation_gain: all_time.total_elevation_gain,
            coverage: answer.coverage,
            year_to_date: Some(this_year),
        },
        coverage_by_source: answer.coverage_by_source,
    }
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
// The athlete's provider profile, totals and counts, their connected
// sources and how fresh each is. A count's breakdown names the gear and
// routes the athlete named.
crate::declare_security!(
    GetAthleteTool => empty,
    reads: ToolData::of(DataClass::Training, &[Stream::Profile]),
);
// All-time and this-year totals: undated, past any grant's window.
crate::declare_security!(
    GetStatsTool => empty,
    reads: ToolData::of(DataClass::Training, &[Stream::ActivityDaily])
        .with_whole_history(),
);

// ============================================================================
// Provider reads and their cache, shared by the two tools
// ============================================================================

/// The profile the cache holds for `cache_key`, if any.
async fn lookup_cached_athlete(cache: &Arc<Cache>, cache_key: &CacheKey) -> Option<Athlete> {
    if let Ok(Some(cached_athlete)) = cache.get::<Athlete>(cache_key).await {
        info!("Cache hit for athlete profile");
        return Some(cached_athlete);
    }
    info!("Cache miss for athlete profile");
    None
}

/// Response metadata for a profile served from a cache.
fn cached_metadata(user_uuid: Uuid, tenant_id: Option<&String>) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
    metadata.insert(
        "tenant_id".to_owned(),
        tenant_id.map_or(Value::Null, |id| Value::String(id.clone())),
    );
    metadata.insert("cached".to_owned(), Value::Bool(true));
    metadata
}

/// Cache athlete profile after fetching from API
async fn cache_athlete_result(cache: &Arc<Cache>, cache_key: &CacheKey, athlete: &Athlete) {
    let ttl = CacheResource::AthleteProfile.recommended_ttl();
    if let Err(e) = cache.set(cache_key, athlete, ttl).await {
        warn!("Failed to cache athlete profile: {}", e);
    } else {
        info!("Cached athlete profile with TTL {:?}", ttl);
    }
}

/// Fetch athlete from API and cache result
///
/// `keys` are the 24 h entry and the last-known copy, both written on success.
async fn fetch_and_cache_athlete(
    provider: &dyn FitnessProvider,
    cache: &Arc<Cache>,
    [cache_key, last_known_key]: [&CacheKey; 2],
    user_uuid: Uuid,
    tenant_id: Option<String>,
    output_format: OutputFormat,
    beside: BesideProfile,
) -> Result<UniversalResponse, ProtocolError> {
    let BesideProfile {
        reader,
        dravr_personal_bests,
    } = beside;
    // The provider's figures are read beside the profile, so they add nothing
    // to its time; each read is bounded by `CONTEXT_READ_BUDGET`.
    let context_read = async {
        match &reader {
            Some(reader) => reader.read(provider).await,
            None => None,
        }
    };
    let (fetched, provider_context) = tokio::join!(provider.get_athlete(), context_read);
    match fetched {
        Ok(athlete) => {
            cache_athlete_result(cache, cache_key, &athlete).await;
            last_known::remember(cache, last_known_key, &athlete).await;

            let mut metadata = HashMap::new();
            metadata.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
            metadata.insert(
                "tenant_id".to_owned(),
                tenant_id.map_or(Value::Null, Value::String),
            );
            metadata.insert("cached".to_owned(), Value::Bool(false));

            formatted_response(
                &GetAthleteResult {
                    athlete,
                    provider_context,
                    dravr_personal_bests,
                },
                output_format,
                metadata,
            )
        }
        Err(e) => Ok(last_known::provider_failure(
            "Failed to fetch athlete profile",
            &e,
        )),
    }
}

/// What a live profile read carries beside the profile.
struct BesideProfile {
    /// The provider's figures, when its backend serves them.
    reader: Option<AthleteContextRead>,
    /// Dravr's own bests.
    dravr_personal_bests: Option<DravrPersonalBests>,
}

/// What it takes to read the provider's own figures about the athlete.
struct AthleteContextRead {
    /// The server's repositories, shared with the runtime: the read moves into
    /// the profile's bounded future, which cannot borrow the tool context.
    repos: Arc<RepositoryRegistry>,
    /// The server's provider registry, shared with the runtime for the same
    /// reason: it names the provider as the athlete knows it.
    registry: Arc<ProviderRegistry>,
    /// The athlete whose gear ledger decides the provider's shoe fallback,
    /// and whose context-read windows the reads count in.
    user_id: Uuid,
    /// The serving backend, never shown to the athlete.
    backend: String,
}

impl AthleteContextRead {
    /// The provider's records, weight and gear, labelled as its own.
    async fn read(&self, provider: &dyn FitnessProvider) -> Option<ProviderAthleteContext> {
        let shoes = ledger_shoes(&self.repos, self.user_id).await;
        let source = source_label(&self.registry, &self.backend);
        // Through a signed-in session, each read counts in the athlete's own
        // context-read windows.
        let budget = self
            .registry
            .session_context_budget(provider.name(), self.user_id);
        let limits = ContextLimits {
            time: CONTEXT_READ_BUDGET,
            budget: budget.as_ref(),
        };
        read_athlete_context(provider, &source, shoes, limits).await
    }
}

/// Whether the athlete's own Dravr gear ledger holds shoes, for the provider's
/// shoe fallback: a ledger that cannot be read is never read as empty. The
/// ledger has no bike kind, so bikes never depend on it.
async fn ledger_shoes(repos: &RepositoryRegistry, user_id: Uuid) -> LedgerShoes {
    match repos.athlete_gear.list_gear(user_id).await {
        Ok(pairs)
            if pairs
                .iter()
                .all(|pair| pair.declaration.kind != GearKind::Shoe) =>
        {
            LedgerShoes::Empty
        }
        Ok(_) => LedgerShoes::Declared,
        Err(e) => {
            warn!(error = %e, "the gear ledger is unreadable; the provider's shoes are not shown");
            LedgerShoes::Unreadable
        }
    }
}

/// Dravr's computed bests for the athlete, labelled as Dravr's. Never fatal:
/// a store that cannot be read leaves the block out. Scoped to the athlete's
/// tenant by the repository.
async fn read_dravr_bests(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<DravrPersonalBests> {
    match repos
        .personal_bests
        .personal_bests(user_id, tenant_id)
        .await
    {
        Ok(bests) => {
            // Each best carries its own run's terms. The profile gate covers
            // only the provider the caller named, so over an external
            // transport a best set on a first-party-only provider's run is
            // dropped here whichever provider was asked for (carnet#1622).
            let bests = ai_scope::retain_served_here(registry, bests, |best| {
                (best.provider.as_str(), None)
            });
            if bests.is_empty() {
                None
            } else {
                Some(DravrPersonalBests {
                    source: DRAVR_BESTS_SOURCE.to_owned(),
                    note: DRAVR_BESTS_NOTE.to_owned(),
                    bests: bests
                        .into_iter()
                        .map(|best| dravr_best(registry, best))
                        .collect(),
                })
            }
        }
        Err(e) => {
            warn!(error = %e, "Dravr's personal bests could not be read; they are left out");
            None
        }
    }
}

fn dravr_best(registry: &ProviderRegistry, best: PersonalBest) -> DravrPersonalBest {
    DravrPersonalBest {
        distance: best.distance,
        elapsed_seconds: best.elapsed_seconds,
        provider: source_label(registry, &best.provider),
        activity_id: best.activity_id,
        achieved_at: best.achieved_at.to_rfc3339(),
    }
}
