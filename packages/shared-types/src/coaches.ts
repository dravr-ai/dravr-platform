// ABOUTME: Shared TypeScript types for AI coaching personas
// ABOUTME: Coach definitions, store types, and version history

// ========== COACH ENUMS ==========

/** Category of a coach */
export type CoachCategory = 'training' | 'nutrition' | 'recovery' | 'recipes' | 'mobility' | 'custom';

/** Visibility setting for coaches */
export type CoachVisibility = 'private' | 'tenant' | 'global';

/** Publish status for store coaches */
export type PublishStatus = 'draft' | 'pending_review' | 'published' | 'rejected';

// ========== DATA REQUIREMENTS ==========

/** Activity data requirements for deterministic pre-fetching */
export interface ActivityDataRequirements {
  /** Number of activities to fetch */
  count: number;
  /** Sport types to filter by (empty = all types) */
  sport_types: string[];
  /** Lookback time frame (e.g., "16w", "90d", "3m") */
  time_frame?: string;
  /** Data detail level */
  mode: 'summary' | 'detailed';
  /** Output format for token efficiency */
  format: 'toon' | 'json';
  /** Analysis type for data sufficiency guidance */
  analysis_type: string;
}

/** Structured data requirements for coach startup context assembly */
export interface DataRequirements {
  /** Activity data to pre-fetch */
  activities?: ActivityDataRequirements;
  /** Whether to also fetch the athlete profile */
  athlete_profile?: boolean;
}

// ========== COACH TYPES ==========

/** A coach (AI coaching persona) */
export interface Coach {
  id: string;
  title: string;
  description: string | null;
  system_prompt: string;
  /** Category - API may return any string value */
  category: string;
  tags: string[];
  token_count: number;
  is_favorite: boolean;
  use_count: number;
  last_used_at: string | null;
  created_at: string;
  updated_at: string;
  is_system: boolean;
  /** Visibility - API may return any string value */
  visibility?: string;
  is_assigned?: boolean;
  is_hidden?: boolean;
  /** ID of source coach if forked */
  forked_from?: string;
  /**
   * Addressable catalogue handle — the `@handle` that invites this coach
   * into a conversation. Present on catalogue coaches and on installed
   * copies; absent on a personal coach that was never published.
   */
  handle?: string;
  /** Query auto-sent on first message to provide analysis context */
  startup_query?: string;
  /** Structured data requirements for deterministic activity pre-fetching */
  data_requirements?: DataRequirements;

  // -- Structured sections (populated for system coaches and structured user coaches) --

  /** Coach purpose extracted from ## Purpose section */
  purpose?: string;
  /** Usage scenarios extracted from ## When to Use section */
  when_to_use?: string;
  /** Core AI instructions extracted from ## Instructions section */
  instructions?: string;
  /** Sample questions extracted from ## Example Inputs section */
  example_inputs?: string;
  /** Response style guidance extracted from ## Example Outputs section */
  example_outputs?: string;
  /** Success definition extracted from ## Success Criteria section */
  success_criteria?: string;
  /** Origin: "contremaitre" (git-managed), "seed" (seeded from files), "custom" (user/admin created) */
  source?: string;

  // -- Personalization (populated only when listing with personalize=true) --

  /** Relevance score in 0..1 for the user's recent sport mix + connected providers */
  match_score?: number;
  /** Whether this coach is in the user's "Recommended for you" set */
  recommended?: boolean;

  /**
   * Per-turn tool-loop iteration budget for this coach. Absent when the coach
   * inherits the tenant-wide `tool_execution.max_iterations` setting.
   */
  max_tool_iterations?: number;
}

// -- Onboarding coach proposal (GET /api/coaches/proposal) --

/** One sport's share of the user's recent activity mix. */
export interface SportShare {
  /** Canonical snake_case sport label (e.g. "run", "ride") */
  sport: string;
  /** Activity count for this sport in the look-back window */
  count: number;
  /** Fraction of total activities this sport represents (0..1) */
  share: number;
}

/** The user's inferred sport profile, shown on the "we analyzed your data" screen. */
export interface SportProfileSummary {
  /** false ⇒ cold start (no provider or no recent activities) */
  has_profile: boolean;
  /** Most-logged sport, if any */
  primary_sport?: string;
  /** Total activities scanned to build the profile */
  total_activities: number;
  /** Look-back window (days) the activities were drawn from */
  window_days: number;
  /** Per-sport breakdown, sorted by count descending */
  sport_mix: SportShare[];
}

/** A coach proposed during onboarding, with its score and a rationale. */
export interface ProposedCoach {
  /** The proposed coach */
  coach: Coach;
  /** Relevance score in 0..1 from the deterministic prefilter */
  match_score: number;
  /** One-sentence, second-person rationale ("why this coach fits you") */
  reason: string;
}

/** Response for GET /api/coaches/proposal. */
export interface CoachProposalResponse {
  /** The inferred sport profile shown before the coach list */
  profile: SportProfileSummary;
  /** Up to 3 proposed coaches, best fit first */
  coaches: ProposedCoach[];
}

/** Request to update an existing coach */
export interface UpdateCoachRequest {
  title?: string;
  description?: string;
  system_prompt?: string;
  /** Category - accepts any valid category string */
  category?: string;
  tags?: string[];
  /** Query auto-sent on first message to provide analysis context */
  startup_query?: string;
  /** Structured data requirements for deterministic activity pre-fetching */
  data_requirements?: DataRequirements;
  /** New `purpose` (if provided) */
  purpose?: string;
  /** New `when_to_use` (if provided) */
  when_to_use?: string;
  /** New `instructions` (if provided) */
  instructions?: string;
  /** New `example_inputs` (if provided) */
  example_inputs?: string;
  /** New `example_outputs` (if provided) */
  example_outputs?: string;
  /** New `success_criteria` (if provided) */
  success_criteria?: string;
  /**
   * New per-turn tool-loop iteration budget. Three-way: omit the key to leave
   * the stored value untouched, send `null` to clear the pin so the coach
   * inherits the tenant-wide `tool_execution.max_iterations` again, or send a
   * number in 1..=50 to pin that budget.
   */
  max_tool_iterations?: number | null;
}

/** Standard metadata for coach API responses */
export interface CoachMetadata {
  timestamp: string;
  api_version: string;
}

/** Response for listing coaches */
export interface ListCoachesResponse {
  coaches: Coach[];
  total: number;
  metadata: CoachMetadata;
}

// ========== COACH STORE TYPES ==========

/** A coach in the public store */
export interface StoreCoach {
  id: string;
  title: string;
  description: string | null;
  /** Category - API may return any string value */
  category: string;
  tags: string[];
  sample_prompts: string[];
  token_count: number;
  install_count: number;
  icon_url: string | null;
  published_at: string | null;
  author_id: string | null;
  /** Addressable catalogue handle (`@handle`), assigned at approval */
  handle?: string;
}

/** Detailed view of a store coach */
export interface StoreCoachDetail extends StoreCoach {
  system_prompt: string;
  created_at: string;
  publish_status: PublishStatus;
}

/** Standard metadata for store API responses */
export interface StoreMetadata {
  timestamp: string;
  api_version: string;
}

/** Response for browsing store coaches */
export interface BrowseCoachesResponse {
  coaches: StoreCoach[];
  next_cursor?: string | null;
  has_more?: boolean;
  total?: number;
  metadata: StoreMetadata;
}

/** Response for searching store coaches */
export interface SearchCoachesResponse {
  coaches: StoreCoach[];
  query: string;
  metadata: StoreMetadata;
}

/** Response for installing a coach */
export interface InstallCoachResponse {
  message: string;
  coach: StoreCoach;
  metadata: StoreMetadata;
}

/** Response for uninstalling a coach */
export interface UninstallCoachResponse {
  message: string;
  source_coach_id: string;
  metadata: StoreMetadata;
}

/** Response for listing installed coaches */
export interface InstallationsResponse {
  coaches: StoreCoach[];
  metadata: StoreMetadata;
}

// ========== COACH ASSIGNMENT TYPES ==========

/** A coach assignment to a user */
export interface CoachAssignment {
  user_id: string;
  user_email?: string;
  assigned_at: string;
  assigned_by?: string;
}

/** Response for assigning a coach */
export interface AssignCoachResponse {
  coach_id: string;
  assigned_count: number;
  total_requested: number;
}

/** Response for unassigning a coach */
export interface UnassignCoachResponse {
  coach_id: string;
  removed_count: number;
  total_requested: number;
}

/** Response for listing assignments */
export interface ListAssignmentsResponse {
  coach_id: string;
  assignments: CoachAssignment[];
}
