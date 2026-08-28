// ABOUTME: Auto-generated TypeScript type definitions for Pierre MCP tool parameters
// ABOUTME: Generated from server tool schemas - DO NOT EDIT MANUALLY
//
// Tool count: 110
// To regenerate: bun run generate (from packages/mcp-types)

/* eslint-disable @typescript-eslint/no-explicit-any */

// ============================================================================
// TOOL PARAMETER TYPES
// ============================================================================

// Note: connect_to_dravr removed - SDK bridge handles authentication locally via RFC 8414 discovery


/**
 * Activate a coach for personalized training guidance
 */
export interface ActivateCoachParams {

  /** ID of the coach to activate */
  coach_id: string;
}


/**
 * Assign a system coach to a specific user (admin only)
 */
export interface AdminAssignCoachParams {

  /** ID of the system coach to assign */
  coach_id: string;

  /** ID of the user to assign the coach to */
  user_id: string;
}


/**
 * Create a new system coach visible to all tenant users (admin only)
 */
export interface AdminCreateSystemCoachParams {

  /** Category: 'training', 'nutrition', 'recovery', 'recipes', 'custom' */
  category?: string;

  /** Description explaining the coach's purpose */
  description?: string;

  /** System prompt that shapes AI responses */
  system_prompt: string;

  /** Tags for filtering and organization */
  tags?: string[];

  /** Display title for the coach */
  title: string;

  /** Visibility: 'tenant' (default) or 'global' */
  visibility?: string;
}


/**
 * Delete a system coach and remove all assignments (admin only)
 */
export interface AdminDeleteSystemCoachParams {

  /** ID of the system coach to delete */
  coach_id: string;
}


/**
 * Get detailed information about a system coach (admin only)
 */
export interface AdminGetSystemCoachParams {

  /** ID of the system coach to retrieve */
  coach_id: string;
}


/**
 * List all assignments for a system coach (admin only)
 */
export interface AdminListCoachAssignmentsParams {

  /** ID of the coach to list assignments for */
  coach_id: string;
}


/**
 * List all system coaches in the tenant (admin only)
 */
export interface AdminListSystemCoachesParams {

  /** Maximum number of coaches to return. Default: 50 */
  limit?: number;

  /** Pagination offset. Default: 0 */
  offset?: number;
}


/**
 * Remove a coach assignment from a user (admin only)
 */
export interface AdminUnassignCoachParams {

  /** ID of the system coach to unassign */
  coach_id: string;

  /** ID of the user to remove the assignment from */
  user_id: string;
}


/**
 * Update an existing system coach (admin only)
 */
export interface AdminUpdateSystemCoachParams {

  /** New category */
  category?: string;

  /** ID of the system coach to update */
  coach_id: string;

  /** New description */
  description?: string;

  /** New system prompt */
  system_prompt?: string;

  /** New tags */
  tags?: string[];

  /** New display title */
  title?: string;
}


/**
 * Perform deep analysis of an individual activity including insights, metrics, and anomaly detection
 */
export interface AnalyzeActivityParams {

  /** ID of the activity to analyze */
  activity_id: string;

  /** Fitness provider name (e.g., 'strava', 'fitbit') */
  provider: string;
}


/**
 * Analyze whether a fitness goal is achievable based on your current fitness level and training history
 */
export interface AnalyzeGoalFeasibilityParams {

  /** Type of goal: 'distance', 'time', 'frequency', or 'performance' */
  goal_type: string;

  /** Fitness provider to analyze. Defaults to configured provider. */
  provider?: string;

  /** Target value for the goal */
  target_value: number;

  /** Number of days to achieve the goal. Default: 30. */
  timeframe_days?: number;
}


/**
 * Analyze nutritional content of a meal from its ingredients
 */
export interface AnalyzeMealNutritionParams {

  /** Array of ingredients with fdc_id and amount_g fields */
  ingredients: {

  /** Amount in grams */
  amount_g: number;

  /** USDA FoodData Central food ID */
  fdc_id: number;
}[];
}


/**
 * Analyze performance trends over time with statistical analysis and insights for a specific metric
 */
export interface AnalyzePerformanceTrendsParams {

  /** Metric to analyze: 'pace', 'speed', 'heart_rate', 'distance', 'duration', 'elevation', 'power' */
  metric: string;

  /** Fitness provider name */
  provider: string;

  /** Time period: 'week', 'month' (default), 'quarter', 'year' */
  timeframe?: string;
}


/**
 * Analyze last night's sleep to generate quality scores and insights. Fetches from a connected provider (WHOOP, Fitbit, Garmin, Terra) automatically
 */
export interface AnalyzeSleepQualityParams {

  /** User's baseline HRV for comparison */
  baseline_hrv?: number;

  /** Array of recent HRV values for trend analysis */
  recent_hrv_values?: number[];

  /** Manual sleep data (used instead of a provider fetch) with fields: duration_hours, deep_sleep_hours, rem_sleep_hours, light_sleep_hours, awake_hours, efficiency_percent, hrv_rmssd_ms */
  sleep_data?: Record<string, any>;

  /** Provider to fetch last night's sleep from (whoop, fitbit, garmin, terra). Omit to auto-select the best connected provider */
  sleep_provider?: string;
}


/**
 * Analyze training load using CTL (chronic training load), ATL (acute training load), and TSB (training stress balance) metrics to assess fitness, fatigue, and form
 */
export interface AnalyzeTrainingLoadParams {

  /** Number of days of history to analyze. Default: 42 (6 weeks). */
  days?: number;

  /** Fitness provider to query (e.g., 'strava'). Defaults to configured provider. */
  provider?: string;

  /** Optional sleep/recovery provider (e.g., 'whoop', 'garmin'). If specified, factors recovery data into training load analysis. */
  sleep_provider?: string;
}


/**
 * Analyze how weather conditions affected activity performance, including temperature, humidity, wind, and precipitation impact
 */
export interface AnalyzeWeatherImpactParams {

  /** ID of the activity to analyze */
  activity_id: string;

  /** Fitness provider to query (e.g., 'strava'). Defaults to configured provider. */
  provider?: string;

  /** Temperature and distance units: 'metric' (default) or 'imperial'. */
  units?: string;
}


/**
 * Browse the Coach Store — the catalogue of PUBLISHED coaches anyone can install. Use this when the athlete asks what coaches exist, or for a coach of a given kind they do not already have. Distinct from `list_coaches`, which lists only the coaches ALREADY in their library. Returns a page plus a `next_cursor` for the following one.
 */
export interface BrowseCoachStoreParams {

  /** Optional category filter: training, nutrition, recovery, recipes, mobility, analysis, or custom. */
  category?: string;

  /** Opaque cursor from a previous call's `next_cursor`, to fetch the next page. */
  cursor?: string;

  /** Coaches per page (1-100, default 20). */
  limit?: number;

  /** Ordering: 'newest' (default), 'popular' (most installed), or 'title'. */
  sort_by?: string;
}


/**
 * Calculate daily calorie and macronutrient needs based on biometrics and goals
 */
export interface CalculateDailyNutritionParams {

  /** Activity level: sedentary, lightly_active, moderately_active, very_active, extra_active */
  activity_level: string;

  /** Age in years */
  age: number;

  /** Gender: male or female */
  gender: string;

  /** Height in centimeters */
  height_cm: number;

  /** Training goal: maintenance, weight_loss, muscle_gain, endurance_performance */
  training_goal: string;

  /** Body weight in kilograms */
  weight_kg: number;
}


/**
 * Calculate an overall fitness score (0-100) based on training consistency, CTL, training volume, and recovery balance
 */
export interface CalculateFitnessScoreParams {

  /** Fitness provider to query. Defaults to configured provider. */
  provider?: string;

  /** Optional sleep/recovery provider (e.g., 'whoop', 'garmin'). If specified, factors recovery quality into fitness score. */
  sleep_provider?: string;
}


/**
 * Calculate advanced fitness metrics for an activity (pace, speed, intensity score, efficiency)
 */
export interface CalculateMetricsParams {

  /** ID of the activity to calculate metrics for */
  activity_id: string;

  /** User age (optional, used to estimate max HR via Fox formula if max_hr not provided) */
  age?: number;

  /** Maximum heart rate (optional, used for intensity calculation) */
  max_hr?: number;

  /** Fitness provider name */
  provider: string;
}


/**
 * Calculate training zones from the athlete's own measurements, falling back to their saved physiology for anything not supplied here. Zone families whose inputs are unknown are listed under `unavailable` instead of being estimated; `input_sources` says where each number came from.
 */
export interface CalculatePersonalizedZonesParams {

  /** Functional Threshold Power in watts. Omit to use the athlete's saved value; power zones are omitted when neither exists. */
  ftp?: number;

  /** Lactate threshold */
  lactate_threshold?: number;

  /** Maximum heart rate in bpm. Omit to use the athlete's saved value, or the Tanaka estimate from their saved age. */
  max_hr?: number;

  /** Resting heart rate in bpm. Omit to use the athlete's saved value. */
  resting_hr?: number;

  /** Sport efficiency factor */
  sport_efficiency?: number;

  /** VO2 max in ml/kg/min. Omit to use the athlete's saved value; pace zones are omitted when neither exists. */
  vo2_max?: number;
}


/**
 * Calculate holistic recovery score combining training stress, sleep, and HRV. Fetches sleep and activity data from connected providers automatically
 */
export interface CalculateRecoveryScoreParams {

  /** Provider to fetch activities for training load (strava, garmin, fitbit, whoop, terra). Omit to auto-select */
  activity_provider?: string;

  /** User's baseline HRV */
  baseline_hrv?: number;

  /** Array of recent HRV values */
  recent_hrv_values?: number[];

  /** Manual sleep data for recovery calculation (used instead of a provider fetch) */
  sleep_data?: Record<string, any>;

  /** Provider to fetch sleep from (whoop, fitbit, garmin, terra). Omit to auto-select the best connected provider */
  sleep_provider?: string;

  /** Training load data with ctl, atl, tsb values (optional) */
  training_load?: Record<string, any>;
}


/**
 * Schedule a future check-in the coach should remember. The reminder is injected into the system prompt of the next coaching conversation. Use when you tell the user 'I'll check back on X tomorrow.'
 */
export interface CoachFollowupScheduleParams {

  /** Coach making the promise. */
  coach_id: string;

  /** Reminder text (≤ 500 characters), e.g., 'check on Achilles pain after long run'. */
  content: string;

  /** Optional originating conversation ID for provenance. */
  conversation_id?: string;

  /** Optional RFC3339 timestamp for when to surface the reminder. */
  due_at?: string;
}


/**
 * Persist a private coach note about the user for the harness memory layer. Use this when you decide that something the user said should be remembered across sessions.
 */
export interface CoachNoteAddParams {

  /** ID of the coach authoring the note. Must match the coach attached to the active conversation. */
  coach_id: string;

  /** Free-form note content (plain text, no markup, ≤ 2000 characters). */
  content: string;

  /** Optional originating conversation ID for provenance. */
  conversation_id?: string;
}


/**
 * Retract a commitment the athlete no longer wants to be held to, so it is never counted or reported. Use when they say they are dropping it or the circumstances changed — injury, travel, a plan revision. Retracting is not failing; do not use this to record that they missed it.
 */
export interface CommitmentCancelParams {

  /** Id of the commitment to retract, as listed in your open-commitments block. */
  commitment_id: string;
}


/**
 * Record a commitment the athlete just made, so it can be checked against their real activity data when the window closes. Call this ONLY after the athlete has agreed to a specific number of sessions by a specific day — if they only said 'ok' to your suggestion, or gave no count or no deadline, ask them to confirm both first and call this on their answer. Do not use it for your own plans or reminders.
 */
export interface CommitmentCreateParams {

  /** Coach the athlete made the promise to. */
  coach_id: string;

  /** Last day that counts, as YYYY-MM-DD in the athlete's local calendar. For 'this week' use the date of the coming Sunday from the current-date anchor. */
  due_date: string;

  /** How many sessions the athlete committed to (1-30). */
  sessions: number;

  /** Optional lowercase sport slug the sessions must match ('run', 'ride', 'swim'). Omit to count any activity. */
  sport?: string;

  /** The promise in the athlete's own terms (≤ 200 characters), e.g. 'three easy runs this week'. */
  statement: string;
}


/**
 * Compare an activity against similar activities, personal bests, or a specific other activity
 */
export interface CompareActivitiesParams {

  /** Primary activity to compare */
  activity_id: string;

  /** Activity ID to compare against (required for 'specific_activity' type) */
  compare_activity_id?: string;

  /** Type of comparison: 'similar_activities' (default), 'pr_comparison', 'specific_activity' */
  comparison_type?: string;

  /** Fitness provider name */
  provider: string;
}


/**
 * Compute and persist Endurance daily training-state rollups for the authenticated user across the requested window — CTL, ATL, TSB (Coggan), acute:chronic load balance, monotony + strain (Foster), ramp rate, and daily TSS load. Use this to seed the `training_history` table on first use, after a long gap in syncs, or when the user asks for a specific historical window. Default window is the last 90 days.
 */
export interface ComputeTrainingHistoryParams {

  /** Inclusive start of the window (ISO date YYYY-MM-DD). Defaults to 90 days before `to` when omitted. */
  from?: string;

  /** Inclusive end of the window (ISO date YYYY-MM-DD). Defaults to today (UTC). */
  to?: string;
}


/**
 * Initiate OAuth connection flow to connect a fitness data provider like Strava, Fitbit, or Garmin
 */
export interface ConnectProviderParams {

  /** Provider to connect (e.g., 'strava', 'fitbit', 'garmin') */
  provider: string;

  /** Optional redirect URL for mobile app OAuth flows (supports dravr://, exp://, http://localhost, https://) */
  redirect_url?: string;
}


/**
 * Create a custom AI coach with personalized training guidance
 */
export interface CreateCoachParams {

  /** Category: training, nutrition, recovery, recipes, custom */
  category?: string;

  /** Description of the coach */
  description?: string;

  /** Example prompts to show users */
  sample_prompts?: string[];

  /** System prompt that shapes AI responses */
  system_prompt: string;

  /** Tags for organization */
  tags?: string[];

  /** Display title for the coach */
  title: string;
}


/**
 * Deactivate the current coach and return to default AI guidance
 */
export interface DeactivateCoachParams {}


/**
 * Delete a coach
 */
export interface DeleteCoachParams {

  /** ID of the coach to delete */
  coach_id: string;
}


/**
 * Delete a fitness configuration by name
 */
export interface DeleteFitnessConfigParams {

  /** Name of the configuration to delete */
  configuration_name: string;

  /** If true, delete user-specific config. If false, delete tenant config (requires admin) */
  user_level?: boolean;
}


/**
 * Delete a recipe from your collection
 */
export interface DeleteRecipeParams {

  /** Recipe ID to delete */
  recipe_id: string;
}


/**
 * Detect training patterns including hard/easy day balance, weekly schedule consistency, volume progression, and overtraining warning signs
 */
export interface DetectPatternsParams {

  /** Fitness provider to query. Defaults to configured provider. */
  provider?: string;

  /** Number of weeks to analyze for patterns. Default: 4. */
  weeks?: number;
}


/**
 * Disconnect from a fitness data provider by removing stored OAuth tokens
 */
export interface DisconnectProviderParams {

  /** Provider to disconnect (e.g., 'strava', 'fitbit', 'garmin') */
  provider: string;
}


/**
 * Discover real named running, cycling, hiking, or ski routes near a location, grounded in OpenStreetMap data via the Overpass API. Pass either a place name via `place` (preferred — e.g. "Prévost, QC" or "Saint-Alexis-des-Monts") or explicit `latitude`/`longitude` coordinates. Place names are forward-geocoded via Nominatim before the Overpass query so the LLM never has to guess coordinates. Use this whenever the user asks you to propose, suggest, or find a route, trail, or outdoor session in a specific area. Never invent street names, trail names, or terrain you have not verified via this tool. Returns up to 20 named routes, nearest first, each with coordinates and a measured `distance_from_center_meters` — quote that distance rather than estimating one from the coordinates. Signed itineraries and trails are listed ahead of paved connectors. An empty list means OSM has no named route in that radius: widen `radius_meters` or say so plainly, never substitute a name you did not read here. For ski queries this reads OSM piste:type data (same source as OpenSkiMap).
 */
export interface DiscoverRoutesParams {

  /** Latitude of the search center, in decimal degrees. Only used when `place` is NOT set. Example: 45.87 for Prévost, Québec. */
  latitude?: number;

  /** Longitude of the search center, in decimal degrees. Only used when `place` is NOT set. Example: -74.08 for Prévost, Québec. */
  longitude?: number;

  /** Place name to resolve via Nominatim forward geocoding. Prefer this over latitude/longitude — pass natural strings like 'Prévost, QC', 'Saint-Alexis-des-Monts', or 'Mont-Tremblat'. When set, the latitude/longitude parameters are ignored. */
  place?: string;

  /** Search radius around the resolved center, in meters. Clamped between 500 and 50000. Defaults to 10000 (10 km). */
  radius_meters?: number;

  /** What kind of route to look for. One of: 'run', 'trail_running', 'ride', 'mountain_bike', 'gravel_ride', 'ebike_ride', 'hike', 'walk', 'cross_country_skiing', 'alpine_skiing', 'backcountry_skiing', 'snowshoe'. Defaults to 'run'. */
  sport_type?: string;
}


/**
 * Export the Endurance 'dossier.json' aggregate for the authenticated user — physiological profile (VO2max, FTP, threshold pace, fitness level), HR + power zones, goals, nutrition, and equipment slots — composed at read time from the underlying tables. Empty slots come back as `null` rather than 404, so coaches can rely on the shape. Mirrors `GET /api/v1/endurance/dossier`.
 */
export interface ExportDossierParams {}


/**
 * Export the Endurance 'intervals.json' shape for a single activity — one row per lap with avg HR, normalized power, intensity factor, and decoupling. Use this when a coach needs the per-interval breakdown for tempo/threshold/VO2max workouts. Activities without laps return a single synthetic interval covering the whole session. Mirrors GET /api/v1/endurance/intervals/{activity_id}.
 */
export interface ExportIntervalsParams {

  /** Provider activity id to analyse (e.g. Strava activity id as string). */
  activity_id: string;
}


/**
 * Export the Endurance 'latest.json' snapshot for the authenticated user — per-activity intensity factor, efficiency factor, variability index, aerobic decoupling, and time-in-zone distribution aggregated over a sliding window. Default window is 7 days; pass `window` (1..=365) to change it. Use this when a coach needs a structured per-activity training-state snapshot grounded in the Endurance deterministic-output contract. The response shape mirrors `GET /api/v1/endurance/latest`.
 */
export interface ExportLatestSnapshotParams {

  /** Window length in days (1..=365). Defaults to 7 when omitted. */
  window?: number;
}


/**
 * Export the Endurance 'routes.json' shape for a single activity — GPX-derived terrain mix (flat/rolling/climb/steep), elevation gain/loss, and distinct climb segments with Strava-style category. Requires the activity stream to include lat/lon and altitude. Mirrors GET /api/v1/endurance/routes/{activity_id}.
 */
export interface ExportRoutesParams {

  /** Provider activity id to analyse (e.g. Strava activity id as string). */
  activity_id: string;
}


/**
 * Return the raw per-second time-series streams for a single activity — heart_rate (bpm), power (watts), cadence (rpm/spm), speed (m/s), altitude (m), and gps_coordinates (lat/lon pairs). Each stream is omitted when the provider didn't record it. Use this when a coach needs the underlying samples to rerun a custom analysis the higher Endurance tools (export_intervals / export_routes) don't cover.
 */
export interface ExtractActivityStreamsParams {

  /** Provider activity id to analyse (e.g. Strava activity id as string). */
  activity_id: string;
}


/**
 * Delete one learned coaching playbook by id (GDPR forget). The athlete can only forget their own playbooks.
 */
export interface ForgetPlaybookParams {

  /** The id of the playbook to forget (from list_coaching_playbooks). */
  playbook_id: string;
}


/**
 * Generate personalized training recommendations
 */
export interface GenerateRecommendationsParams {

  /** Fitness provider to query (e.g., 'strava'). Defaults to configured provider. */
  provider?: string;

  /** Type of recommendations to generate: 'all' (default), 'training_plan', 'recovery', 'intensity', 'goal_specific', or 'nutrition'. */
  recommendation_type?: string;
}


/**
 * Get the currently active coach
 */
export interface GetActiveCoachParams {}


/**
 * Retrieve user's fitness activities from connected providers. For a specific year or date range (e.g. '2022 races'), pass `after`/`before` epoch-second bounds — do NOT page recent activities via `limit` to reach old data. Use `sort_by` to honor an explicit ordering request (e.g. longest-to-shortest). Supports sport-type filtering and pagination.
 */
export interface GetActivitiesParams {

  /** Unix timestamp (epoch seconds) lower bound — return only activities at or after this time. REQUIRED for any year- or date-scoped question (e.g. 'my 2022 races', 'runs last spring'): set `after` to the start of the range and `before` to the end. Do NOT try to reach older activities by raising `limit` — the feed is newest-first and will not page back far; date filters are the only way to query history. Deep historical windows are served from cache or fetched in the background, so a first request may return a 'fetching, ask again shortly' status. */
  after?: number;

  /** Unix timestamp (epoch seconds) upper bound — return only activities at or before this time. Pair with `after` to bound a specific year or date range (e.g. all of 2022 = after 1640995200, before 1672531200). */
  before?: number;

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;

  /** Maximum number of activities to return (1-400). Use the smallest value that answers the question: 1 for 'last activity', 5-10 for 'this week', 20 for broader queries. Do NOT raise this to reach older/historical activities — the feed is newest-first; use `after`/`before` date bounds instead. Response includes has_more and pagination info for follow-up requests. */
  limit?: number;

  /** Output mode: 'summary' (default, minimal fields) or 'detailed' (full activity data). */
  mode?: string;

  /** Number of activities to skip for pagination. */
  offset?: number;

  /** Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider. */
  provider?: string;

  /** Order of the returned list, applied BEFORE `limit` so the truncated set keeps the right activities. One of: 'date_desc' (default, newest first), 'date_asc' (oldest first), 'distance_desc' (longest first), 'distance_asc' (shortest first), 'duration_desc' (longest time first), 'duration_asc' (shortest time first). Map the user's wording: 'de la plus longue à la plus courte' / 'longest to shortest' => distance_desc; 'oldest first' / 'du début' => date_asc. */
  sort_by?: string;

  /** Filter by sport type (e.g., 'run', 'ride', 'swim'). Case-insensitive. */
  sport_type?: string;
}


/**
 * Get AI-powered intelligence insights and recommendations for a specific activity
 */
export interface GetActivityIntelligenceParams {

  /** ID of the activity to analyze */
  activity_id: string;

  /** Fitness provider name (e.g., 'strava'). Defaults to configured provider. */
  provider: string;
}


/**
 * Retrieve the user's athlete profile from connected fitness providers including personal details and preferences
 */
export interface GetAthleteParams {

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;

  /** Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider. */
  provider?: string;
}


/**
 * Get detailed information about a specific coach
 */
export interface GetCoachParams {

  /** ID of the coach to retrieve */
  coach_id: string;
}


/**
 * Get the complete catalog of available configuration options
 */
export interface GetConfigurationCatalogParams {}


/**
 * Get available configuration profile templates
 */
export interface GetConfigurationProfilesParams {}


/**
 * Check the connection status of fitness data providers. If no provider is specified, returns status for all supported providers.
 */
export interface GetConnectionStatusParams {

  /** Optional: specific provider to check (e.g., 'strava'). If omitted, checks all providers. */
  provider?: string;
}


/**
 * Check how fresh the user's fitness data is across all connected providers. Returns last sync time and freshness level for each provider. Use this to decide if a refresh is needed before answering data-dependent questions.
 */
export interface GetDataFreshnessParams {}


/**
 * Get fitness configuration for the current user (falls back to tenant default)
 */
export interface GetFitnessConfigParams {

  /** Name of the configuration to retrieve (default: 'default') */
  configuration_name?: string;
}


/**
 * Get detailed nutritional information for a specific food item
 */
export interface GetFoodDetailsParams {

  /** USDA FoodData Central ID of the food item */
  fdc_id: number;
}


/**
 * Fetch a CONSENTING group member's recent or past activities. This is the ONLY way to read a peer's data in a group chat — `get_activities` always returns YOUR own data, never a peer's. Identify the member by their roster display name. For a specific past race or date range, pass `after`/`before` epoch-second bounds. Returns an error (not data) if the member has not shared their data via `/group consent yes`.
 */
export interface GetGroupMemberActivitiesParams {

  /** Optional Unix epoch-second lower bound. Pair with `before` to target a specific past race or date range. */
  after?: number;

  /** Optional Unix epoch-second upper bound. */
  before?: number;

  /** Max activities to return (1-100, default 30). */
  limit?: number;

  /** Display name of the group member whose activities to fetch, as shown in the group roster (e.g. 'Raph'). Required. */
  member: string;

  /** Optional sport filter (e.g. 'ride', 'run'). */
  sport_type?: string;
}


/**
 * Get stored health snapshots (body composition, vitals)
 */
export interface GetHealthSnapshotsParams {

  /** End of the date range as RFC3339 timestamp. Defaults to now. */
  end?: string;

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;

  /** Start of the date range as RFC3339 timestamp. Defaults to 30 days ago. */
  start?: string;
}


/**
 * Get optimal nutrient timing recommendations around workouts
 */
export interface GetNutrientTimingParams {

  /** Daily protein target in grams */
  daily_protein_g: number;

  /** Body weight in kilograms */
  weight_kg: number;

  /** Workout intensity: low, moderate, high */
  workout_intensity: string;
}


/**
 * Get details of a specific recipe
 */
export interface GetRecipeParams {

  /** Recipe ID */
  recipe_id: string;
}


/**
 * Get macro targets and constraints for LLM recipe generation
 */
export interface GetRecipeConstraintsParams {

  /** Target calories for the meal */
  calories?: number;

  /** Dietary restrictions like gluten_free, vegan */
  dietary_restrictions?: string[];

  /** Maximum cooking time */
  max_cook_time_mins?: number;

  /** Maximum preparation time */
  max_prep_time_mins?: number;

  /** pre_training, post_training, rest_day, or general */
  meal_timing?: string;

  /** User's Total Daily Energy Expenditure */
  tdee?: number;
}


/**
 * Get stored recovery and readiness metrics
 */
export interface GetRecoveryMetricsParams {

  /** End of the date range as RFC3339 timestamp. Defaults to now. */
  end?: string;

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;

  /** Start of the date range as RFC3339 timestamp. Defaults to 30 days ago. */
  start?: string;
}


/**
 * Get stored sleep sessions from the database
 */
export interface GetSleepSessionsParams {

  /** End of the date range as RFC3339 timestamp. Defaults to now. */
  end?: string;

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;

  /** Start of the date range as RFC3339 timestamp. Defaults to 30 days ago. */
  start?: string;
}


/**
 * Retrieve aggregated activity statistics from a connected fitness provider. The top-level total_* fields are ALL-TIME / lifetime totals. When the provider supplies it (currently Strava), a `year_to_date` object holds CURRENT-CALENDAR-YEAR totals — use that for 'this year' / annual questions and never report the all-time totals as annual. If `year_to_date` is absent, the provider does not expose annual figures. IMPORTANT: Strava's ride and run totals here count ONLY the base sport type and EXCLUDE variant disciplines (VirtualRide, GravelRide, MountainBikeRide, EBikeRide; TrailRun, VirtualRun), so they undercount multi-discipline athletes. For a true cross-discipline total (e.g. 'total km cycling this year'), do NOT report this single ride/run figure — call get_activities for the period and sum distance across all related sport types.
 */
export interface GetStatsParams {

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;

  /** Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider. */
  provider?: string;
}


/**
 * Get detailed information about a specific stretching exercise
 */
export interface GetStretchingExerciseParams {

  /** The unique ID of the stretching exercise */
  exercise_id: string;
}


/**
 * Fetch persisted Endurance daily training-state rollups for the authenticated user across the requested window. Returns chronological CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load rows. Use this BEFORE prescribing new load so coaching advice can cite the framework values (Coggan / Foster) on every numeric claim per the Endurance deterministic-output rule. Interpret TSB relative to CTL (form as % of fitness), and report `acwr` strictly as the magnitude of 7-day load against the 28-day baseline — a descriptive ratio, not a predictor of harm. Default window is the last 90 days.
 */
export interface GetTrainingHistoryParams {

  /** Inclusive start of the window (ISO date YYYY-MM-DD). Defaults to 90 days before `to` when omitted. */
  from?: string;

  /** Inclusive end of the window (ISO date YYYY-MM-DD). Defaults to today (UTC). */
  to?: string;
}


/**
 * Fetch the athlete's active training plan: goal race, block strategy, and the day-by-day weeks. Use before answering any 'what's my plan / what am I doing this week' question — the stored plan, not memory of the conversation, is the source of truth. The calendar block lists what Dravr has on the athlete's Intervals.icu calendar (each entry's prescription_id is what prescribe_workout's replaces and withdraw_prescribed_workout take) and whether push_training_plan would change it.
 */
export interface GetTrainingPlanParams {

  /** Coach persona slug asking; falls back to the athlete's coach-agnostic plan. */
  coach_id?: string;

  /** Include superseded week versions (the adjustment audit trail). */
  include_history?: boolean;
}


/**
 * Get your current training configuration settings
 */
export interface GetUserConfigurationParams {}


/**
 * Get the weather forecast (temperature in °C, conditions, humidity, wind) for a location and date, grounded in the Open-Meteo forecast API. Use this whenever the user asks you to base a session on the temperature or weather, or proposes an activity today or on an upcoming date. Pass either a place name via `place` (preferred — e.g. "Prévost, QC") or explicit `latitude`/`longitude`. Optionally pass `date` (YYYY-MM-DD, defaults to today UTC) and `hour` (0-23 UTC, defaults to 12). Forecast coverage is up to ~16 days ahead. Never invent the temperature — call this tool to obtain it.
 */
export interface GetWeatherForecastParams {

  /** Target date as YYYY-MM-DD (UTC). Defaults to today. Must be within ~16 days for forecast coverage. */
  date?: string;

  /** Hour of day 0-23 (UTC). Defaults to 12 (midday). */
  hour?: number;

  /** Latitude in decimal degrees. Only used when `place` is NOT set. */
  latitude?: number;

  /** Longitude in decimal degrees. Only used when `place` is NOT set. */
  longitude?: number;

  /** Place name to resolve via Nominatim forward geocoding. Prefer this over latitude/longitude — pass natural strings like 'Prévost, QC'. When set, latitude/longitude are ignored. */
  place?: string;
}


/**
 * Get detailed information about a specific yoga pose
 */
export interface GetYogaPoseParams {

  /** The unique ID of the yoga pose */
  pose_id: string;
}


/**
 * Hide a coach from listings
 */
export interface HideCoachParams {

  /** ID of the coach to hide */
  coach_id: string;
}


/**
 * Install a published Coach Store coach into the athlete's own library, creating their personal copy. Call it only once the athlete has asked for that specific coach — pass the `id` from `browse_coach_store` or `search_coach_store`. After installing, `activate_coach` makes it the coach that answers.
 */
export interface InstallCoachFromStoreParams {

  /** UUID of the published coach to install, as returned by `browse_coach_store` or `search_coach_store`. Required. */
  coach_id: string;
}


/**
 * List available AI coaches for personalized training guidance
 */
export interface ListCoachesParams {

  /** Filter by category */
  category?: string;

  /** Only show favorites. Default: false */
  favorites_only?: boolean;

  /** Include system coaches. Default: true */
  include_system?: boolean;

  /** Max results. Default: 50 */
  limit?: number;

  /** Pagination offset. Default: 0 */
  offset?: number;
}


/**
 * List the coaching playbooks the harness has learned for this athlete — the trigger→intervention patterns and how well each has worked (success/failure counts + confidence). Use this to tell the athlete what you have learned about what works for them.
 */
export interface ListCoachingPlaybooksParams {

  /** Max playbooks to return (1-50, default 12). */
  limit?: number;
}


/**
 * List connected data sources (devices and providers)
 */
export interface ListDataSourcesParams {

  /** Output format: 'json' (default) or 'toon' (token-efficient for LLMs). */
  format?: string;
}


/**
 * List all available fitness configuration names for the user and tenant
 */
export interface ListFitnessConfigsParams {

  /** Include tenant-level configurations (default: true) */
  include_tenant?: boolean;
}


/**
 * List all hidden coaches
 */
export interface ListHiddenCoachesParams {}


/**
 * List your saved recipes
 */
export interface ListRecipesParams {

  /** Maximum results (default: 20) */
  limit?: number;

  /** Filter by meal timing */
  meal_timing?: string;

  /** Pagination offset */
  offset?: number;
}


/**
 * List stretching exercises with optional filtering by category, difficulty, muscle group, or activity type
 */
export interface ListStretchingExercisesParams {

  /** Filter by recommended activity (e.g., running, cycling, swimming) */
  activity_type?: string;

  /** Filter by stretch category: static, dynamic, pnf, ballistic */
  category?: string;

  /** Filter by difficulty: beginner, intermediate, advanced */
  difficulty?: string;

  /** Maximum number of results to return (default: 20) */
  limit?: number;

  /** Filter by muscle group (e.g., hamstrings, quadriceps, calves) */
  muscle_group?: string;
}


/**
 * List the Endurance cornerstone workout templates: long_run_z2, threshold_4x8, vo2_5x3, recovery_30min, tempo_progression, sweet_spot_2x20. Each row carries the structured steps + target zones the prescribe_workout tool will push to the user's Intervals.icu calendar.
 */
export interface ListWorkoutTemplatesParams {}


/**
 * List yoga poses with optional filtering by category, difficulty, pose type, or recovery context
 */
export interface ListYogaPosesParams {

  /** Filter by pose category: standing, seated, supine, prone, inversion, balance, twist */
  category?: string;

  /** Filter by difficulty: beginner, intermediate, advanced */
  difficulty?: string;

  /** Maximum number of results to return (default: 20) */
  limit?: number;

  /** Filter by muscle group (e.g., hamstrings, hips, shoulders) */
  muscle_group?: string;

  /** Filter by pose type: stretch, strength, balance, relaxation, breathing */
  pose_type?: string;

  /** Filter by recovery context: post_cardio, rest_day, morning, evening */
  recovery_context?: string;
}


/**
 * Get personalized sleep schedule recommendations based on training and recovery needs
 */
export interface OptimizeSleepScheduleParams {

  /** Training load data (ctl, atl, tsb) */
  training_load?: Record<string, any>;

  /** Wake time in HH:MM format (default: 06:00) */
  typical_wake_time?: string;

  /** low, moderate, or high */
  upcoming_workout_intensity?: string;
}


/**
 * Predict future performance based on training
 */
export interface PredictPerformanceParams {

  /** Fitness provider to query (e.g., 'strava'). Defaults to configured provider. */
  provider?: string;

  /** Target sport for performance prediction (e.g., 'Run', 'Ride', 'Swim'). Default: 'Run'. */
  target_sport?: string;
}


/**
 * Write one workout onto the athlete's Intervals.icu calendar for a given date, and record it in the prescribed_workouts ledger. Requires a connected Intervals.icu account. Pass EITHER template_slug — one of the cornerstones (long_run_z2, threshold_4x8, vo2_5x3, recovery_30min, tempo_progression, sweet_spot_2x20) or a session you prescribed this athlete before — OR session, a structured session you authored for anything those do not express. Args: date (YYYY-MM-DD), template_slug or session, optional coach_id, optional replaces. Without replaces every call adds a new calendar entry; with replaces = a prescription_id (from an earlier call, or from get_training_plan's calendar block) that entry is changed in place instead. withdraw_prescribed_workout removes one.
 */
export interface PrescribeWorkoutParams {

  /** Optional coach id stamped onto the audit row. */
  coach_id?: string;

  /** Calendar date the workout is scheduled for (YYYY-MM-DD). */
  date: string;

  /** prescription_id of an earlier prescription to change in place — the calendar entry keeps its slot and gets this workout. Omit to add a new entry. */
  replaces?: string;

  /** A structured session you authored, for anything the cornerstones do not express. */
  session?: {

  /** One of: polarized, threshold, vo2max, recovery, pyramid. */
  intensity_distribution: string;

  /** Session name as you state it to the athlete. */
  name: string;

  /** Sport: run, ride, swim, walk, hike, ski, yoga, … */
  sport: string;

  /** The steps in order. Total duration is summed from them. */
  structure: {

  /** Distance in metres for a distance-based step. */
  distance_meters?: number;

  /** How long ONE repetition of this step lasts, in seconds. */
  duration_seconds: number;

  /** What this step is ("Warm-up", "Montées", "Interval", "Cool-down"). */
  label: string;

  /** Coaching cue for this step, in your own voice — it reaches the athlete's calendar entry. */
  note?: string;

  /** Repetitions of this step; omit for a single block. */
  repeat?: number;

  /** Intensity RELATIVE to the athlete's thresholds ("Z2", "Z2 HR", "Threshold", "sweet spot", "88-93% FTP"). Never absolute watts. */
  target_zone: string;
}[];
};

  /** Slug of a stored template: one of the six cornerstones, or a session you prescribed this athlete before. Use `session` instead for anything new. */
  template_slug?: string;
}


/**
 * Put the athlete's active training plan on their Intervals.icu calendar, or bring the calendar up to date after the plan changed: creates the days that are missing, updates the ones that changed, removes the ones the plan no longer has, and leaves alone any the athlete edited on Intervals.icu. Never touches dates before today. Call it when the athlete or coach asks to put or update the plan on their calendar — not on your own initiative after a save; save_training_plan's reply says when the calendar is behind. Requires a saved plan and a connected Intervals.icu account. Args: optional coach_id, optional from_date.
 */
export interface PushTrainingPlanParams {

  /** Coach persona slug whose plan to push; falls back to the athlete's coach-agnostic plan. */
  coach_id?: string;

  /** First date to push (YYYY-MM-DD). Defaults to today in the athlete's calendar, and is never earlier than that: dates already past are not rewritten. */
  from_date?: string;
}


/**
 * Retrieve stored facts the harness has remembered about the user. Returns recent facts ordered by last-update timestamp. Use this when you need to confirm what you already know before answering.
 */
export interface RecallUserMemoryParams {

  /** Optional coach scope. When set, only facts attached to this coach are returned. */
  coach_id?: string;

  /** Optional fact kind filter (preference | physiology | injury | goal | schedule | equipment | other). */
  kind?: string;

  /** Maximum facts to return (1–50, default 12). */
  limit?: number;
}


/**
 * Trigger a data refresh from a connected fitness provider. Use when the user's data seems outdated, when they ask about recent activities that aren't showing, or when they explicitly request a sync. Set wait=true to block until sync completes.
 */
export interface RefreshProviderDataParams {

  /** Provider to refresh: 'strava', 'garmin', 'whoop', 'fitbit', or 'all' to refresh all connected providers. */
  provider: string;

  /** Why the refresh is needed (for logging). E.g., 'user asked about today's run but latest activity is from 3 days ago'. */
  reason?: string;

  /** If true, wait for the sync to complete before returning. If false (default), start sync in background and return immediately. Use true when you need fresh data to answer the user's question. */
  wait?: boolean;
}


/**
 * Persist a structured durable fact about the user (preference, physiology, injury, goal, schedule, equipment, other). Use this when the user explicitly confirms something the coach should remember next time.
 */
export interface RememberFactParams {

  /** Coach attaching the fact (optional; defaults to user-wide). */
  coach_id?: string;

  /** Confidence in [0.0, 1.0]. Direct user confirmation → 0.9+. */
  confidence: number;

  /** One of: preference | physiology | injury | goal | schedule | equipment | other. */
  kind: string;

  /** Asserted value or detail. */
  object: string;

  /** Short verb phrase (prefers, has, runs, targets, avoids, ...). */
  predicate: string;

  /** Short subject phrase, usually 'you'. */
  subject: string;
}


/**
 * Save a validated recipe to your collection
 */
export interface SaveRecipeParams {

  /** Recipe description */
  description?: string;

  /** Array of {name, amount, unit} */
  ingredients: {

  /** Quantity */
  amount: number;

  /** Ingredient name */
  name: string;

  /** Unit of measurement */
  unit: string;
}[];

  /** Array of instruction steps */
  instructions: string[];

  /** pre_training, post_training, rest_day, or general */
  meal_timing?: string;

  /** Recipe name */
  name: string;

  /** Number of servings */
  servings: number;

  /** Tags for organization */
  tags?: string[];
}


/**
 * Persist the training plan you agreed with the athlete — outline (goal race, blocks, strategy) and/or day-by-day weeks — in the SAME turn you state it. Saved plans are re-injected into future conversations; an unsaved plan is forgotten. Adjustments re-save only the changed week(s) and supersede prospectively; past weeks stay immutable. For a day with interval structure, give steps (same shape as prescribe_workout's session.structure) — that is what puts workout-builder steps and a planned load on the calendar; prose alone reaches it as a timed entry. Saving never writes to the athlete's calendar: when the reply's calendar.stale is true, their Intervals.icu calendar no longer matches the plan — tell them and offer push_training_plan.
 */
export interface SaveTrainingPlanParams {

  /** Coach persona slug saving the plan. */
  coach_id?: string;

  /** Originating conversation ID for provenance. */
  conversation_id?: string;

  /** Existing pillar Goal fact this plan serves, when known. */
  goal_fact_id?: string;

  /** The plan outline (goal race + strategy, optionally blocks). Required when creating a plan; omit to adjust weeks of the existing active plan. Re-sending an outline supersedes the athlete's current plan. */
  outline?: {

  /** Ordered training blocks from now to the goal race. Omit for a short plan that has no mesocycle structure. */
  blocks?: {

  /** What this block is for, in coach voice. */
  intent: string;

  /** One of: rest | base | build | peak | taper. */
  phase: string;

  /** Block start date, YYYY-MM-DD. */
  start: string;

  /** Optional target weekly volume in hours. */
  target_hours?: number;

  /** Block length in weeks. */
  weeks: number;
}[];

  /** The goal (A) race this plan builds toward. */
  goal_race: {

  /** Race date, YYYY-MM-DD. */
  date: string;

  /** Discipline: gravel, xco, road, trail run, … */
  discipline: string;

  /** Race name as the athlete calls it. */
  name: string;

  /** A = goal race, B = tune-up, C = training race. */
  priority: string;
};

  /** Other races on the calendar (B/C priorities). */
  races?: {

  /** Race date, YYYY-MM-DD. */
  date: string;

  /** Discipline: gravel, xco, road, trail run, … */
  discipline: string;

  /** Race name as the athlete calls it. */
  name: string;

  /** A = goal race, B = tune-up, C = training race. */
  priority: string;
}[];

  /** The coach's strategy in prose — what the athlete sees as the long-term direction. */
  strategy: string;
};

  /** Day-by-day weeks to save. Send the full multi-week detail when the athlete asks to see the whole plan; send a single adjusted week for 'move Tuesday to Wednesday' changes. */
  weeks?: {

  /** Why this week is being re-saved; omit on first save. */
  adjustment_reason?: string;

  /** The day rows, in date order (max 7). */
  days: {

  /** Day date, YYYY-MM-DD. */
  date: string;

  /** Planned duration in minutes; omit for rest days, and omit when steps are given — it is summed from them. */
  duration_min?: number;

  /** Intensity RELATIVE to thresholds ('Z2', 'tempo', '88-93% FTP'). Never absolute watts. With steps, the day's summary label. */
  intensity?: string;

  /** Sport (mtb, gravel, run, …) or 'rest'. */
  sport: string;

  /** The session's steps, in order — the same shape as prescribe_workout's session.structure. Give them for any day with interval structure (warm-up, work, recovery, cool-down; repeat on the work and recovery steps): that is what puts workout-builder steps and a planned load on the athlete's calendar. Prose alone reaches the calendar as a timed entry. Omit for a steady or unstructured day. */
  steps?: {

  /** Distance in metres for a distance-based step. */
  distance_meters?: number;

  /** How long ONE repetition of this step lasts, in seconds. */
  duration_seconds: number;

  /** What this step is ("Warm-up", "Montées", "Interval", "Cool-down"). */
  label: string;

  /** Coaching cue for this step, in your own voice — it reaches the athlete's calendar entry. */
  note?: string;

  /** Repetitions of this step; omit for a single block. */
  repeat?: number;

  /** Intensity RELATIVE to the athlete's thresholds ("Z2", "Z2 HR", "Threshold", "sweet spot", "88-93% FTP"). Never absolute watts. */
  target_zone: string;
}[];

  /** What to do, in coach voice. */
  workout: string;
}[];

  /** The week's intent in one line. */
  focus?: string;

  /** Date of the week's first day, YYYY-MM-DD. */
  week_start: string;
}[];
}


/**
 * Search the Coach Store for PUBLISHED coaches matching a phrase, e.g. 'ultra trail' or 'vegetarian nutrition'. Searches the whole marketplace, unlike `search_coaches`, which searches only the athlete's own library. Install a result with `install_coach_from_store`.
 */
export interface SearchCoachStoreParams {

  /** Max results (1-100, default 20). */
  limit?: number;

  /** Text to match against a published coach's title, description or tags. Required. */
  query: string;
}


/**
 * Search for coaches by query. Returns up to 20 results by default. Check the `has_more` field before requesting additional results with offset.
 */
export interface SearchCoachesParams {

  /** Filter by category */
  category?: string;

  /** Maximum results per request. Default: 20, max: 100 */
  limit?: number;

  /** Pagination offset. Default: 0. Only use if previous response had has_more=true */
  offset?: number;

  /** Search query */
  query: string;
}


/**
 * Search USDA FoodData Central database for foods. Returns up to 10 results by default. Check the `has_more` field before requesting additional pages.
 */
export interface SearchFoodParams {

  /** Page number (1-indexed, default: 1). Only use if previous response had has_more=true */
  page_number?: number;

  /** Number of results per page (default: 10, max: 50) */
  page_size?: number;

  /** Search query for food items */
  query: string;
}


/**
 * Search your recipes by name, tags, or description
 */
export interface SearchRecipesParams {

  /** Maximum results (default: 10) */
  limit?: number;

  /** Pagination offset */
  offset?: number;

  /** Search query */
  query: string;
}


/**
 * Save or update fitness configuration for the current user or tenant
 */
export interface SetFitnessConfigParams {

  /** Fitness configuration object with sport_types, intelligence settings */
  config: Record<string, any>;

  /** Name for this configuration (default: 'default') */
  configuration_name?: string;

  /** If true, save as user-specific config. If false, save as tenant default (requires admin) */
  user_level?: boolean;
}


/**
 * Create a new fitness goal with specified type, target value, and timeframe
 */
export interface SetGoalParams {

  /** Type of goal: 'distance', 'time', 'frequency', or 'performance' */
  goal_type: string;

  /** Sport type for the goal (e.g., 'Running', 'Cycling'). Default: 'Running' */
  sport?: string;

  /** Target value for the goal (km for distance, sessions for frequency, etc.) */
  target_value: number;

  /** Goal timeframe: 'week', 'month', 'quarter', or 'year'. Default: 'month' */
  timeframe?: string;

  /** Title or description for the goal */
  title?: string;
}


/**
 * Save the athlete's physiological measurements — FTP, threshold pace, max and resting heart rate, lactate threshold, VO2 max, weight, age — so training load, zones and every personalised calculation use their real numbers instead of generic per-sport estimates. Call this whenever the athlete states one of these values, for example 'my FTP is 285' or 'my max HR is 190'. Pass only the fields they actually gave you; everything else keeps its stored value. The result is the profile re-read from storage after the write, so report back only what it contains.
 */
export interface SetPhysiologyParams {

  /** Age in years. */
  age?: number;

  /** One of beginner, recreational, intermediate, advanced, elite, professional. */
  fitness_level?: string;

  /** Functional Threshold Power in watts. Saving it also derives and stores the athlete's power zones. */
  ftp_watts?: number;

  /** Lactate threshold as a fraction of maximum heart rate, between 0.65 and 0.95. Combined with max_hr this gives the LTHR that heart-rate-based training load uses. */
  lactate_threshold_percentage?: number;

  /** Maximum heart rate in bpm. Saving it together with resting_hr derives and stores the athlete's heart-rate zones. */
  max_hr?: number;

  /** The athlete's main sport, e.g. run, ride, swim, trail_running. */
  primary_sport?: string;

  /** Resting heart rate in bpm. */
  resting_hr?: number;

  /** Threshold pace in seconds per kilometre — the running equivalent of FTP. A 4:10/km threshold is 250. */
  threshold_pace_sec_per_km?: number;

  /** Years of structured training experience. */
  training_experience_years?: number;

  /** VO2 max in ml/kg/min. */
  vo2_max?: number;

  /** Body weight in kilograms. */
  weight?: number;
}


/**
 * Show a previously hidden coach
 */
export interface ShowCoachParams {

  /** ID of the coach to show */
  coach_id: string;
}


/**
 * Get AI-suggested fitness goals based on your activity history and fitness level
 */
export interface SuggestGoalsParams {

  /** Fitness provider to analyze. Defaults to configured provider. */
  provider?: string;
}


/**
 * Get AI-powered recommendation on whether to rest or train. Fetches sleep and activity data from connected providers automatically
 */
export interface SuggestRestDayParams {

  /** Provider to fetch activities for training load (strava, garmin, fitbit, whoop, terra). Omit to auto-select */
  activity_provider?: string;

  /** User's baseline HRV */
  baseline_hrv?: number;

  /** Recent HRV values for trend analysis */
  recent_hrv_values?: number[];

  /** Manual sleep data (used instead of a provider fetch) */
  sleep_data?: Record<string, any>;

  /** Provider to fetch last night's sleep from (whoop, fitbit, garmin, terra). Omit to auto-select the best connected provider */
  sleep_provider?: string;

  /** Training load data (ctl, atl, tsb) */
  training_load?: Record<string, any>;
}


/**
 * Get personalized stretching recommendations based on your recent activity type
 */
export interface SuggestStretchesForActivityParams {

  /** The activity type to get stretches for (e.g., running, cycling, swimming, hiking) */
  activity_type: string;

  /** Optional focus: warmup (dynamic stretches) or cooldown (static stretches) */
  focus?: string;

  /** Maximum number of stretches to suggest (default: 6) */
  limit?: number;
}


/**
 * Create a personalized yoga sequence for recovery based on your recent activities or goals
 */
export interface SuggestYogaSequenceParams {

  /** Maximum difficulty level: beginner, intermediate, advanced */
  difficulty?: string;

  /** Target duration in minutes (10, 15, 20, 30). Default: 15 */
  duration_minutes?: number;

  /** Optional muscle/area focus: hips, hamstrings, back, shoulders */
  focus_area?: string;

  /** Purpose of the sequence: post_cardio, rest_day, morning, evening, stress_relief */
  purpose: string;
}


/**
 * Toggle the favorite status of a coach
 */
export interface ToggleCoachFavoriteParams {

  /** ID of the coach */
  coach_id: string;
}


/**
 * Track progress toward a specific fitness goal with milestone achievements and projections
 */
export interface TrackProgressParams {

  /** ID of the goal to track progress for */
  goal_id: string;

  /** Fitness provider to query. Defaults to configured provider. */
  provider?: string;
}


/**
 * Analyze sleep patterns over time to identify trends and insights. Fetches history from a connected provider (WHOOP, Fitbit, Garmin, Terra) automatically
 */
export interface TrackSleepTrendsParams {

  /** Days of sleep history to fetch from the provider (default 7) */
  days?: number;

  /** Array of sleep data objects (minimum 7 days) */
  sleep_history?: {

  /** Date of sleep record */
  date: string;

  /** Sleep duration in hours */
  duration_hours: number;
}[];

  /** Provider to fetch sleep history from (whoop, fitbit, garmin, terra). Omit to auto-select the best connected provider */
  sleep_provider?: string;
}


/**
 * Update an existing coach's settings
 */
export interface UpdateCoachParams {

  /** New category */
  category?: string;

  /** ID of the coach to update */
  coach_id: string;

  /** New description */
  description?: string;

  /** New system prompt */
  system_prompt?: string;

  /** New tags */
  tags?: string[];

  /** New title */
  title?: string;
}


/**
 * Update your training configuration settings
 */
export interface UpdateUserConfigurationParams {

  /** Configuration parameters to update */
  parameters?: Record<string, any>;

  /** Profile name to apply */
  profile?: string;
}


/**
 * Validate configuration parameters for physiological correctness
 */
export interface ValidateConfigurationParams {

  /** Configuration parameters to validate */
  parameters: Record<string, any>;
}


/**
 * Validate recipe nutrition using USDA FoodData Central
 */
export interface ValidateRecipeParams {

  /** Array of {name, amount, unit} */
  ingredients: {

  /** Quantity */
  amount: number;

  /** Ingredient name */
  name: string;

  /** Unit of measurement */
  unit: string;
}[];

  /** Recipe name */
  name?: string;

  /** Number of servings */
  servings: number;
}


/**
 * Check a factual claim against the claim-verification pipeline (rhetoric → deterministic bounds → personalized physiology → evidence corpus) and return a structured verdict with evidence strength and optional citations. Use this before emitting any physiological, nutrition, training, or supplement claim you are not certain about.
 */
export interface VerifyClaimParams {

  /** One of: physiological | training_prescription | nutrition | recovery | supplement | injury_rehab. */
  category: string;

  /** The factual proposition to verify, as a single sentence. */
  claim: string;

  /** Optional coach id for the verdict row. */
  coach_id?: string;

  /** Optional conversation id to attach the verdict row to. */
  conversation_id?: string;

  /** Minimum evidence strength that counts as Supported. One of: strong | mixed | weak | none. Defaults to mixed. */
  minimum_strength?: string;
}


/**
 * Remove a workout that prescribe_workout wrote to the athlete's Intervals.icu calendar: deletes the calendar entry and marks the prescription withdrawn. Only for single prescriptions — an entry the training plan put there is removed by adjusting the plan (save_training_plan) and pushing it (push_training_plan). Args: prescription_id.
 */
export interface WithdrawPrescribedWorkoutParams {

  /** prescription_id of the entry to remove — from the prescribe_workout reply, or from get_training_plan's calendar block. */
  prescription_id: string;
}

// ============================================================================
// TOOL RESPONSE TYPES
// ============================================================================

/**
 * Generic MCP tool response wrapper
 */
export interface McpToolResponse {
  content?: Array<{
    type: string;
    text?: string;
    [key: string]: any;
  }>;
  isError?: boolean;
  [key: string]: any;
}

/**
 * MCP error response
 */
export interface McpErrorResponse {
  code: number;
  message: string;
  data?: any;
}


// ============================================================================
// TOOL NAME TYPES
// ============================================================================

/**
 * Union type of all available tool names
 */
export type ToolName = "activate_coach" | "admin_assign_coach" | "admin_create_system_coach" | "admin_delete_system_coach" | "admin_get_system_coach" | "admin_list_coach_assignments" | "admin_list_system_coaches" | "admin_unassign_coach" | "admin_update_system_coach" | "analyze_activity" | "analyze_goal_feasibility" | "analyze_meal_nutrition" | "analyze_performance_trends" | "analyze_sleep_quality" | "analyze_training_load" | "analyze_weather_impact" | "browse_coach_store" | "calculate_daily_nutrition" | "calculate_fitness_score" | "calculate_metrics" | "calculate_personalized_zones" | "calculate_recovery_score" | "coach_followup_schedule" | "coach_note_add" | "commitment_cancel" | "commitment_create" | "compare_activities" | "compute_training_history" | "connect_provider" | "create_coach" | "deactivate_coach" | "delete_coach" | "delete_fitness_config" | "delete_recipe" | "detect_patterns" | "disconnect_provider" | "discover_routes" | "export_dossier" | "export_intervals" | "export_latest_snapshot" | "export_routes" | "extract_activity_streams" | "forget_playbook" | "generate_recommendations" | "get_active_coach" | "get_activities" | "get_activity_intelligence" | "get_athlete" | "get_coach" | "get_configuration_catalog" | "get_configuration_profiles" | "get_connection_status" | "get_data_freshness" | "get_fitness_config" | "get_food_details" | "get_group_member_activities" | "get_health_snapshots" | "get_nutrient_timing" | "get_recipe" | "get_recipe_constraints" | "get_recovery_metrics" | "get_sleep_sessions" | "get_stats" | "get_stretching_exercise" | "get_training_history" | "get_training_plan" | "get_user_configuration" | "get_weather_forecast" | "get_yoga_pose" | "hide_coach" | "install_coach_from_store" | "list_coaches" | "list_coaching_playbooks" | "list_data_sources" | "list_fitness_configs" | "list_hidden_coaches" | "list_recipes" | "list_stretching_exercises" | "list_workout_templates" | "list_yoga_poses" | "optimize_sleep_schedule" | "predict_performance" | "prescribe_workout" | "push_training_plan" | "recall_user_memory" | "refresh_provider_data" | "remember_fact" | "save_recipe" | "save_training_plan" | "search_coach_store" | "search_coaches" | "search_food" | "search_recipes" | "set_fitness_config" | "set_goal" | "set_physiology" | "show_coach" | "suggest_goals" | "suggest_rest_day" | "suggest_stretches_for_activity" | "suggest_yoga_sequence" | "toggle_coach_favorite" | "track_progress" | "track_sleep_trends" | "update_coach" | "update_user_configuration" | "validate_configuration" | "validate_recipe" | "verify_claim" | "withdraw_prescribed_workout";

/**
 * Map of tool names to their parameter types
 */
export interface ToolParamsMap {
  "activate_coach": ActivateCoachParams;
  "admin_assign_coach": AdminAssignCoachParams;
  "admin_create_system_coach": AdminCreateSystemCoachParams;
  "admin_delete_system_coach": AdminDeleteSystemCoachParams;
  "admin_get_system_coach": AdminGetSystemCoachParams;
  "admin_list_coach_assignments": AdminListCoachAssignmentsParams;
  "admin_list_system_coaches": AdminListSystemCoachesParams;
  "admin_unassign_coach": AdminUnassignCoachParams;
  "admin_update_system_coach": AdminUpdateSystemCoachParams;
  "analyze_activity": AnalyzeActivityParams;
  "analyze_goal_feasibility": AnalyzeGoalFeasibilityParams;
  "analyze_meal_nutrition": AnalyzeMealNutritionParams;
  "analyze_performance_trends": AnalyzePerformanceTrendsParams;
  "analyze_sleep_quality": AnalyzeSleepQualityParams;
  "analyze_training_load": AnalyzeTrainingLoadParams;
  "analyze_weather_impact": AnalyzeWeatherImpactParams;
  "browse_coach_store": BrowseCoachStoreParams;
  "calculate_daily_nutrition": CalculateDailyNutritionParams;
  "calculate_fitness_score": CalculateFitnessScoreParams;
  "calculate_metrics": CalculateMetricsParams;
  "calculate_personalized_zones": CalculatePersonalizedZonesParams;
  "calculate_recovery_score": CalculateRecoveryScoreParams;
  "coach_followup_schedule": CoachFollowupScheduleParams;
  "coach_note_add": CoachNoteAddParams;
  "commitment_cancel": CommitmentCancelParams;
  "commitment_create": CommitmentCreateParams;
  "compare_activities": CompareActivitiesParams;
  "compute_training_history": ComputeTrainingHistoryParams;
  "connect_provider": ConnectProviderParams;
  "create_coach": CreateCoachParams;
  "deactivate_coach": DeactivateCoachParams;
  "delete_coach": DeleteCoachParams;
  "delete_fitness_config": DeleteFitnessConfigParams;
  "delete_recipe": DeleteRecipeParams;
  "detect_patterns": DetectPatternsParams;
  "disconnect_provider": DisconnectProviderParams;
  "discover_routes": DiscoverRoutesParams;
  "export_dossier": ExportDossierParams;
  "export_intervals": ExportIntervalsParams;
  "export_latest_snapshot": ExportLatestSnapshotParams;
  "export_routes": ExportRoutesParams;
  "extract_activity_streams": ExtractActivityStreamsParams;
  "forget_playbook": ForgetPlaybookParams;
  "generate_recommendations": GenerateRecommendationsParams;
  "get_active_coach": GetActiveCoachParams;
  "get_activities": GetActivitiesParams;
  "get_activity_intelligence": GetActivityIntelligenceParams;
  "get_athlete": GetAthleteParams;
  "get_coach": GetCoachParams;
  "get_configuration_catalog": GetConfigurationCatalogParams;
  "get_configuration_profiles": GetConfigurationProfilesParams;
  "get_connection_status": GetConnectionStatusParams;
  "get_data_freshness": GetDataFreshnessParams;
  "get_fitness_config": GetFitnessConfigParams;
  "get_food_details": GetFoodDetailsParams;
  "get_group_member_activities": GetGroupMemberActivitiesParams;
  "get_health_snapshots": GetHealthSnapshotsParams;
  "get_nutrient_timing": GetNutrientTimingParams;
  "get_recipe": GetRecipeParams;
  "get_recipe_constraints": GetRecipeConstraintsParams;
  "get_recovery_metrics": GetRecoveryMetricsParams;
  "get_sleep_sessions": GetSleepSessionsParams;
  "get_stats": GetStatsParams;
  "get_stretching_exercise": GetStretchingExerciseParams;
  "get_training_history": GetTrainingHistoryParams;
  "get_training_plan": GetTrainingPlanParams;
  "get_user_configuration": GetUserConfigurationParams;
  "get_weather_forecast": GetWeatherForecastParams;
  "get_yoga_pose": GetYogaPoseParams;
  "hide_coach": HideCoachParams;
  "install_coach_from_store": InstallCoachFromStoreParams;
  "list_coaches": ListCoachesParams;
  "list_coaching_playbooks": ListCoachingPlaybooksParams;
  "list_data_sources": ListDataSourcesParams;
  "list_fitness_configs": ListFitnessConfigsParams;
  "list_hidden_coaches": ListHiddenCoachesParams;
  "list_recipes": ListRecipesParams;
  "list_stretching_exercises": ListStretchingExercisesParams;
  "list_workout_templates": ListWorkoutTemplatesParams;
  "list_yoga_poses": ListYogaPosesParams;
  "optimize_sleep_schedule": OptimizeSleepScheduleParams;
  "predict_performance": PredictPerformanceParams;
  "prescribe_workout": PrescribeWorkoutParams;
  "push_training_plan": PushTrainingPlanParams;
  "recall_user_memory": RecallUserMemoryParams;
  "refresh_provider_data": RefreshProviderDataParams;
  "remember_fact": RememberFactParams;
  "save_recipe": SaveRecipeParams;
  "save_training_plan": SaveTrainingPlanParams;
  "search_coach_store": SearchCoachStoreParams;
  "search_coaches": SearchCoachesParams;
  "search_food": SearchFoodParams;
  "search_recipes": SearchRecipesParams;
  "set_fitness_config": SetFitnessConfigParams;
  "set_goal": SetGoalParams;
  "set_physiology": SetPhysiologyParams;
  "show_coach": ShowCoachParams;
  "suggest_goals": SuggestGoalsParams;
  "suggest_rest_day": SuggestRestDayParams;
  "suggest_stretches_for_activity": SuggestStretchesForActivityParams;
  "suggest_yoga_sequence": SuggestYogaSequenceParams;
  "toggle_coach_favorite": ToggleCoachFavoriteParams;
  "track_progress": TrackProgressParams;
  "track_sleep_trends": TrackSleepTrendsParams;
  "update_coach": UpdateCoachParams;
  "update_user_configuration": UpdateUserConfigurationParams;
  "validate_configuration": ValidateConfigurationParams;
  "validate_recipe": ValidateRecipeParams;
  "verify_claim": VerifyClaimParams;
  "withdraw_prescribed_workout": WithdrawPrescribedWorkoutParams;
}
