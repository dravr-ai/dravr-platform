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
}

/** Response when forking a coach */
export interface ForkCoachResponse {
  coach: Coach;
  source_coach_id: string;
}

/** Request to create a new coach */
export interface CreateCoachRequest {
  title: string;
  description?: string;
  system_prompt: string;
  /** Category - accepts any valid category string */
  category: string;
  tags?: string[];
  visibility?: string;
  /** Query auto-sent on first message to provide analysis context */
  startup_query?: string;
  /** Structured data requirements for deterministic activity pre-fetching */
  data_requirements?: DataRequirements;
  /** Coach purpose (from ## Purpose section) */
  purpose?: string;
  /** Usage scenarios (from ## When to Use section) */
  when_to_use?: string;
  /** Core AI instructions (from ## Instructions section) */
  instructions?: string;
  /** Sample questions (from ## Example Inputs section) */
  example_inputs?: string;
  /** Response style guidance (from ## Example Outputs section) */
  example_outputs?: string;
  /** Success definition (from ## Success Criteria section) */
  success_criteria?: string;
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

// ========== COACH VERSION HISTORY ==========

/** A version of a coach (for history tracking) */
export interface CoachVersion {
  version: number;
  content_snapshot: Record<string, unknown>;
  change_summary: string | null;
  created_at: string;
  created_by_name: string | null;
}

/** Response for listing coach versions */
export interface ListVersionsResponse {
  versions: CoachVersion[];
  current_version: number;
  total: number;
}

/** Response for reverting to a previous version */
export interface RevertVersionResponse {
  coach: Coach;
  reverted_to_version: number;
  new_version: number;
}

/** A field change in a coach diff */
export interface FieldChange {
  field: string;
  old_value: unknown | null;
  new_value: unknown | null;
}

/** Response for coach diff between versions */
export interface CoachDiffResponse {
  from_version: number;
  to_version: number;
  changes: FieldChange[];
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

/** Count of coaches in a category */
export interface CategoryCount {
  category: string;
  name: string;
  count: number;
}

/** Response for store categories */
export interface CategoriesResponse {
  categories: CategoryCount[];
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

// ========== COACH IMPORT/EXPORT TYPES ==========

/** Parsed fields from a coach markdown preview */
export interface ParsedCoachFields {
  name: string;
  title: string;
  category: string;
  tags: string[];
  purpose: string;
  has_instructions: boolean;
  has_example_inputs: boolean;
  has_example_outputs: boolean;
  has_success_criteria: boolean;
}

/** Response for importing a coach from markdown */
export interface ImportCoachResponse {
  coach: Coach;
  parsed_name: string;
  token_count: number;
  warnings?: string[];
}

/** Response for previewing a coach import */
export interface ImportPreviewResponse {
  valid: boolean;
  parsed?: ParsedCoachFields;
  errors?: string[];
  warnings?: string[];
  content_hash?: string;
  duplicate_exists: boolean;
  duplicate_coach_id?: string;
  token_count?: number;
}

/** Request body for importing a coach from a URL */
export interface ImportFromUrlRequest {
  url: string;
  save?: boolean;
}
