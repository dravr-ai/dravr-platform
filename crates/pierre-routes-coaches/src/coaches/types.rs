// ABOUTME: Request and response types for Coaches REST API endpoints
// ABOUTME: Contains all serializable structs for coach CRUD, versioning, admin, and store operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::coaches::DataRequirements;
use pierre_database::database::{
    coaches::{
        Coach, CoachAssignment as DbCoachAssignment, CoachCategory, CoachListItem, CoachVersion,
        CoachVisibility, CreateCoachRequest,
        CreateSystemCoachRequest as DbCreateSystemCoachRequest, UpdateCoachRequest,
    },
    store_listings::CoachWithListing,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================
// Core Coach Response Types
// ============================================

/// Response for a coach
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CoachResponse {
    /// Unique identifier
    pub id: String,
    /// Display title
    pub title: String,
    /// Optional description
    pub description: Option<String>,
    /// System prompt that shapes AI responses
    pub system_prompt: String,
    /// Category for organization
    pub category: String,
    /// Tags for filtering
    pub tags: Vec<String>,
    /// Estimated token count
    pub token_count: u32,
    /// Whether marked as favorite
    pub is_favorite: bool,
    /// Number of times used
    pub use_count: u32,
    /// Last time used
    pub last_used_at: Option<String>,
    /// Creation timestamp
    pub created_at: String,
    /// Last update timestamp
    pub updated_at: String,
    /// Whether this is a system coach (admin-created)
    pub is_system: bool,
    /// Visibility level
    pub visibility: String,
    /// Whether this coach is assigned to the current user
    pub is_assigned: bool,
    /// ID of the coach this was forked from (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Whether prerequisites are met (only present if `check_prerequisites=true`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prerequisites_met: Option<bool>,
    /// List of missing prerequisites (only present if `check_prerequisites=true`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_prerequisites: Option<Vec<MissingPrerequisite>>,
    /// Query auto-sent on first message to provide analysis context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_query: Option<String>,
    /// Structured data requirements for deterministic activity pre-fetching
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_requirements: Option<DataRequirements>,
    /// Coach purpose (from ## Purpose section)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Usage scenarios (from ## When to Use section)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// Core AI instructions (from ## Instructions section)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Sample questions (from ## Example Inputs section)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_inputs: Option<String>,
    /// Response style guidance (from ## Example Outputs section)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_outputs: Option<String>,
    /// Success definition (from ## Success Criteria section)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_criteria: Option<String>,
}

/// A missing prerequisite for a coach
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MissingPrerequisite {
    /// Type of prerequisite (provider, `activity_count`, `activity_type`)
    pub prerequisite_type: String,
    /// The specific requirement (e.g., "strava", "50 activities", "Run")
    pub requirement: String,
    /// Human-readable message explaining what's missing
    pub message: String,
}

impl From<Coach> for CoachResponse {
    fn from(coach: Coach) -> Self {
        Self {
            id: coach.id.to_string(),
            title: coach.title,
            description: coach.description,
            system_prompt: coach.system_prompt,
            category: coach.category.as_str().to_owned(),
            tags: coach.tags,
            token_count: coach.token_count,
            is_favorite: false, // Defaults; preferences live in coach_assignments
            use_count: 0,
            last_used_at: None,
            created_at: coach.created_at.to_rfc3339(),
            updated_at: coach.updated_at.to_rfc3339(),
            is_system: coach.is_system,
            visibility: coach.visibility.as_str().to_owned(),
            is_assigned: false, // Default for single coach responses
            forked_from: coach.forked_from,
            prerequisites_met: None,
            missing_prerequisites: None,
            startup_query: coach.startup_query,
            data_requirements: coach.data_requirements,
            purpose: coach.purpose,
            when_to_use: coach.when_to_use,
            instructions: coach.instructions,
            example_inputs: coach.example_inputs,
            example_outputs: coach.example_outputs,
            success_criteria: coach.success_criteria,
        }
    }
}

impl From<CoachListItem> for CoachResponse {
    fn from(item: CoachListItem) -> Self {
        Self {
            id: item.coach.id.to_string(),
            title: item.coach.title,
            description: item.coach.description,
            system_prompt: item.coach.system_prompt,
            category: item.coach.category.as_str().to_owned(),
            tags: item.coach.tags,
            token_count: item.coach.token_count,
            is_favorite: item.is_favorite,
            use_count: item.use_count,
            last_used_at: item.last_used_at.map(|dt| dt.to_rfc3339()),
            created_at: item.coach.created_at.to_rfc3339(),
            updated_at: item.coach.updated_at.to_rfc3339(),
            is_system: item.coach.is_system,
            visibility: item.coach.visibility.as_str().to_owned(),
            is_assigned: item.is_assigned,
            forked_from: item.coach.forked_from,
            prerequisites_met: None,
            missing_prerequisites: None,
            startup_query: item.coach.startup_query,
            data_requirements: item.coach.data_requirements,
            purpose: item.coach.purpose,
            when_to_use: item.coach.when_to_use,
            instructions: item.coach.instructions,
            example_inputs: item.coach.example_inputs,
            example_outputs: item.coach.example_outputs,
            success_criteria: item.coach.success_criteria,
        }
    }
}

/// Response for listing coaches
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListCoachesResponse {
    /// List of coaches
    pub coaches: Vec<CoachResponse>,
    /// Total count of coaches matching the filter
    pub total: u32,
    /// Metadata
    pub metadata: CoachesMetadata,
}

/// Metadata for coaches response
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CoachesMetadata {
    /// Response timestamp
    pub timestamp: String,
    /// API version
    pub api_version: String,
}

/// Query parameters for listing coaches
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListCoachesQuery {
    /// Filter by category
    pub category: Option<String>,
    /// Filter to favorites only
    pub favorites_only: Option<bool>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
    /// Include system coaches (default: true)
    pub include_system: Option<bool>,
    /// Include hidden coaches (default: false)
    pub include_hidden: Option<bool>,
    /// Check prerequisites against user's connected providers (default: false)
    pub check_prerequisites: Option<bool>,
}

/// Query parameters for searching coaches
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchCoachesQuery {
    /// Search query string
    pub q: String,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Pagination offset
    pub offset: Option<u32>,
}

/// Response for toggle favorite
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToggleFavoriteResponse {
    /// New favorite status
    pub is_favorite: bool,
}

/// Response for record usage
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RecordUsageResponse {
    /// Whether the usage was recorded
    pub success: bool,
}

/// Response for hide/show coach operations
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct HideCoachResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// Whether the coach is now hidden (true) or visible (false)
    pub is_hidden: bool,
}

/// Response for forking a coach
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ForkCoachResponse {
    /// The newly created forked coach
    pub coach: CoachResponse,
    /// The ID of the original coach that was forked
    pub source_coach_id: String,
}

/// Response for importing a coach from markdown
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImportCoachResponse {
    /// The created coach
    pub coach: CoachResponse,
    /// The parsed name/slug from the markdown
    pub parsed_name: String,
    /// Estimated token count from the markdown
    pub token_count: u32,
    /// Import warnings (missing optional sections, high token count, etc.)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Response for previewing a coach import without saving
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImportPreviewResponse {
    /// Whether the markdown parsed successfully
    pub valid: bool,
    /// Parsed coach fields (present when valid)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedCoachFields>,
    /// Parse errors (present when invalid)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Warnings about missing optional sections or quality issues
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Content hash for deduplication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Whether a coach with this content already exists for the user
    pub duplicate_exists: bool,
    /// ID of the existing duplicate coach (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_coach_id: Option<String>,
    /// Estimated token count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
}

/// Parsed coach fields extracted from markdown for preview
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ParsedCoachFields {
    /// Coach name/slug from frontmatter
    pub name: String,
    /// Display title
    pub title: String,
    /// Category
    pub category: String,
    /// Tags
    pub tags: Vec<String>,
    /// Purpose section content
    pub purpose: String,
    /// Whether instructions section is present
    pub has_instructions: bool,
    /// Whether `example_inputs` section is present
    pub has_example_inputs: bool,
    /// Whether `example_outputs` section is present
    pub has_example_outputs: bool,
    /// Whether `success_criteria` section is present
    pub has_success_criteria: bool,
}

/// Request body for importing a coach from a URL
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImportFromUrlBody {
    /// HTTPS URL pointing to a markdown coach definition
    pub url: String,
    /// Whether to save the imported coach (true) or just preview (false)
    #[serde(default = "default_save_true")]
    pub save: bool,
}

const fn default_save_true() -> bool {
    true
}

// ============================================
// Create/Update Request Types
// ============================================

/// Request body for creating a coach (mirrors `CreateCoachRequest` with serde derives)
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateCoachBody {
    /// Display title for the coach
    pub title: String,
    /// Optional description explaining the coach's purpose
    pub description: Option<String>,
    /// System prompt that shapes AI responses
    pub system_prompt: String,
    /// Category for organization
    pub category: Option<String>,
    /// Tags for filtering and search
    #[serde(default)]
    pub tags: Vec<String>,
    /// Sample prompts for quick-start suggestions
    #[serde(default)]
    pub sample_prompts: Vec<String>,
    /// Query auto-sent on first message to provide analysis context
    pub startup_query: Option<String>,
    /// Structured data requirements for deterministic activity pre-fetching
    pub data_requirements: Option<DataRequirements>,
    /// Coach purpose (from ## Purpose section)
    pub purpose: Option<String>,
    /// Usage scenarios (from ## When to Use section)
    pub when_to_use: Option<String>,
    /// Core AI instructions (from ## Instructions section)
    pub instructions: Option<String>,
    /// Sample questions (from ## Example Inputs section)
    pub example_inputs: Option<String>,
    /// Response style guidance (from ## Example Outputs section)
    pub example_outputs: Option<String>,
    /// Success definition (from ## Success Criteria section)
    pub success_criteria: Option<String>,
}

impl From<CreateCoachBody> for CreateCoachRequest {
    fn from(body: CreateCoachBody) -> Self {
        Self {
            title: body.title,
            description: body.description,
            system_prompt: body.system_prompt,
            category: body
                .category
                .map(|c| CoachCategory::parse(&c))
                .unwrap_or_default(),
            tags: body.tags,
            sample_prompts: body.sample_prompts,
            startup_query: body.startup_query,
            data_requirements: body.data_requirements,
            purpose: body.purpose,
            when_to_use: body.when_to_use,
            instructions: body.instructions,
            example_inputs: body.example_inputs,
            example_outputs: body.example_outputs,
            success_criteria: body.success_criteria,
        }
    }
}

/// Request body for updating a coach
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateCoachBody {
    /// New title (if provided)
    pub title: Option<String>,
    /// New description (if provided)
    pub description: Option<String>,
    /// New system prompt (if provided)
    pub system_prompt: Option<String>,
    /// New category (if provided)
    pub category: Option<String>,
    /// New tags (if provided)
    pub tags: Option<Vec<String>>,
    /// New sample prompts (if provided)
    pub sample_prompts: Option<Vec<String>>,
    /// New startup query (if provided)
    pub startup_query: Option<String>,
    /// New data requirements (if provided)
    pub data_requirements: Option<DataRequirements>,
    /// New `purpose` (if provided)
    pub purpose: Option<String>,
    /// New `when_to_use` (if provided)
    pub when_to_use: Option<String>,
    /// New `instructions` (if provided)
    pub instructions: Option<String>,
    /// New `example_inputs` (if provided)
    pub example_inputs: Option<String>,
    /// New `example_outputs` (if provided)
    pub example_outputs: Option<String>,
    /// New `success_criteria` (if provided)
    pub success_criteria: Option<String>,
}

impl From<UpdateCoachBody> for UpdateCoachRequest {
    fn from(body: UpdateCoachBody) -> Self {
        Self {
            title: body.title,
            description: body.description,
            system_prompt: body.system_prompt,
            category: body.category.map(|c| CoachCategory::parse(&c)),
            tags: body.tags,
            sample_prompts: body.sample_prompts,
            startup_query: body.startup_query,
            data_requirements: body.data_requirements,
            purpose: body.purpose,
            when_to_use: body.when_to_use,
            instructions: body.instructions,
            example_inputs: body.example_inputs,
            example_outputs: body.example_outputs,
            success_criteria: body.success_criteria,
        }
    }
}

// ============================================
// Coach Generation Types
// ============================================

/// Request to generate a coach from a conversation
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GenerateCoachRequest {
    /// The conversation ID to analyze
    pub conversation_id: String,
    /// Maximum number of messages to analyze (default: 10)
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,
}

const fn default_max_messages() -> usize {
    10
}

/// Response for coach generation
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GenerateCoachResponse {
    /// Generated title for the coach
    pub title: String,
    /// Generated description
    pub description: String,
    /// Generated system prompt
    pub system_prompt: String,
    /// Suggested category
    pub category: String,
    /// Suggested tags
    pub tags: Vec<String>,
    /// Number of messages analyzed
    pub messages_analyzed: usize,
    /// Total messages in the conversation
    pub total_messages: usize,
}

/// Internal struct for parsing LLM JSON response
#[derive(Debug, Deserialize)]
pub(super) struct GeneratedCoachData {
    pub title: String,
    pub description: String,
    pub system_prompt: String,
    pub category: String,
    pub tags: Vec<String>,
}

// ============================================
// Version History Response Types
// ============================================

/// Response for a coach version
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CoachVersionResponse {
    /// Version number
    pub version: i32,
    /// Full content snapshot
    pub content_snapshot: serde_json::Value,
    /// Summary of what changed
    pub change_summary: Option<String>,
    /// When this version was created
    pub created_at: String,
    /// Name of the user who created this version
    pub created_by_name: Option<String>,
}

impl From<CoachVersion> for CoachVersionResponse {
    fn from(v: CoachVersion) -> Self {
        Self {
            version: v.version,
            content_snapshot: v.content_snapshot,
            change_summary: v.change_summary,
            created_at: v.created_at.to_rfc3339(),
            created_by_name: None, // Populated separately with user lookup
        }
    }
}

/// Response for listing coach versions
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListVersionsResponse {
    /// List of versions
    pub versions: Vec<CoachVersionResponse>,
    /// Current version number
    pub current_version: i32,
    /// Total number of versions
    pub total: usize,
}

/// Response for reverting to a version
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RevertVersionResponse {
    /// The coach after reversion
    pub coach: CoachResponse,
    /// The version that was reverted to
    pub reverted_to_version: i32,
    /// The new version number (after revert)
    pub new_version: i32,
}

/// Response for comparing two versions
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CoachDiffResponse {
    /// Source version number
    pub from_version: i32,
    /// Target version number
    pub to_version: i32,
    /// List of field changes
    pub changes: Vec<FieldChange>,
}

/// A single field change between versions
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FieldChange {
    /// Name of the field that changed
    pub field: String,
    /// Old value (None if field was added)
    pub old_value: Option<serde_json::Value>,
    /// New value (None if field was removed)
    pub new_value: Option<serde_json::Value>,
}

/// Query parameters for listing versions
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListVersionsQuery {
    /// Maximum number of versions to return
    pub limit: Option<u32>,
}

// ============================================
// Admin Request/Response Types
// ============================================

/// Request body for creating a system coach
#[derive(Debug, Deserialize)]
pub struct AdminCreateCoachBody {
    /// Display title for the coach
    pub title: String,
    /// Optional description explaining the coach's purpose
    pub description: Option<String>,
    /// System prompt that shapes AI responses
    pub system_prompt: String,
    /// Category for organization
    pub category: Option<String>,
    /// Tags for filtering and search
    #[serde(default)]
    pub tags: Vec<String>,
    /// Sample prompts for quick-start suggestions
    #[serde(default)]
    pub sample_prompts: Vec<String>,
    /// Visibility level (tenant or global)
    pub visibility: Option<String>,
}

impl From<AdminCreateCoachBody> for DbCreateSystemCoachRequest {
    fn from(body: AdminCreateCoachBody) -> Self {
        Self {
            title: body.title,
            description: body.description,
            system_prompt: body.system_prompt,
            category: body
                .category
                .map(|c| CoachCategory::parse(&c))
                .unwrap_or_default(),
            tags: body.tags,
            sample_prompts: body.sample_prompts,
            visibility: body
                .visibility
                .map_or(CoachVisibility::Tenant, |v| CoachVisibility::parse(&v)),
        }
    }
}

/// Request body for assigning/unassigning coaches
#[derive(Debug, Deserialize)]
pub struct AssignCoachBody {
    /// User IDs to assign/unassign
    pub user_ids: Vec<String>,
}

/// Response for coach assignment
#[derive(Debug, Serialize)]
pub struct AssignCoachResponse {
    /// Coach ID
    pub coach_id: String,
    /// Number of users successfully assigned
    pub assigned_count: usize,
    /// Total number of users requested
    pub total_requested: usize,
}

/// Response for coach unassignment
#[derive(Debug, Serialize)]
pub struct UnassignCoachResponse {
    /// Coach ID
    pub coach_id: String,
    /// Number of users successfully unassigned
    pub removed_count: usize,
    /// Total number of users requested
    pub total_requested: usize,
}

/// Coach assignment info
#[derive(Debug, Serialize)]
pub struct CoachAssignment {
    /// User ID
    pub user_id: String,
    /// User email (for display)
    pub user_email: Option<String>,
    /// When assigned
    pub assigned_at: String,
    /// Who assigned
    pub assigned_by: Option<String>,
}

impl From<DbCoachAssignment> for CoachAssignment {
    fn from(db: DbCoachAssignment) -> Self {
        Self {
            user_id: db.user_id,
            user_email: db.user_email,
            assigned_at: db.assigned_at,
            assigned_by: db.assigned_by,
        }
    }
}

/// Response for listing assignments
#[derive(Debug, Serialize)]
pub struct ListAssignmentsResponse {
    /// Coach ID
    pub coach_id: String,
    /// List of assignments
    pub assignments: Vec<CoachAssignment>,
}

// ============================================
// Store Admin Request/Response Types
// ============================================

/// Query parameters for store listing endpoints
#[derive(Debug, Deserialize)]
pub struct StoreListParams {
    /// Maximum number of results
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
    /// Sort by: "newest" or `most_installed`
    pub sort_by: Option<String>,
}

/// Store admin statistics response
#[derive(Debug, Serialize)]
pub struct StoreAdminStatsResponse {
    /// Number of coaches pending review
    pub pending_count: u32,
    /// Number of published coaches
    pub published_count: u32,
    /// Number of rejected coaches
    pub rejected_count: u32,
    /// Total installs across all published coaches
    pub total_installs: u32,
    /// Rejection rate as percentage
    pub rejection_rate: f64,
}

/// Store coach response with author email
#[derive(Debug, Serialize)]
pub struct StoreCoachResponse {
    /// Coach ID
    pub id: String,
    /// Display title
    pub title: String,
    /// Optional description
    pub description: Option<String>,
    /// System prompt
    pub system_prompt: String,
    /// Category
    pub category: String,
    /// Tags
    pub tags: Vec<String>,
    /// Sample prompts
    pub sample_prompts: Vec<String>,
    /// Token count
    pub token_count: u32,
    /// Install count
    pub install_count: u32,
    /// Icon URL
    pub icon_url: Option<String>,
    /// Published timestamp
    pub published_at: Option<String>,
    /// When submitted for review
    pub submitted_at: Option<String>,
    /// When review decision was made
    pub rejected_at: Option<String>,
    /// Author user ID
    pub author_id: Option<String>,
    /// Author email (joined from users table)
    pub author_email: Option<String>,
    /// Rejection reason (if rejected)
    pub rejection_reason: Option<String>,
    /// Rejection notes (parsed from `rejection_reason`)
    pub rejection_notes: Option<String>,
    /// Creation timestamp
    pub created_at: String,
    /// Publish status
    pub publish_status: String,
}

impl StoreCoachResponse {
    /// Create from `CoachWithListing` with author email
    pub(super) fn from_coach_with_listing(
        cwl: CoachWithListing,
        author_email: Option<String>,
    ) -> Self {
        let coach = cwl.coach;
        let listing = cwl.listing;

        // Parse rejection reason into reason code and notes
        let (rejection_reason, rejection_notes) =
            listing
                .rejection_reason
                .as_ref()
                .map_or((None, None), |reason| {
                    reason.find(": ").map_or_else(
                        || (Some(reason.clone()), None),
                        |colon_pos| {
                            let code = reason[..colon_pos].to_owned();
                            let notes = reason[colon_pos + 2..].to_owned();
                            (Some(code), Some(notes))
                        },
                    )
                });

        Self {
            id: coach.id.to_string(),
            title: coach.title,
            description: coach.description,
            system_prompt: coach.system_prompt,
            category: coach.category.as_str().to_owned(),
            tags: coach.tags,
            sample_prompts: coach.sample_prompts,
            token_count: coach.token_count,
            install_count: listing.install_count,
            icon_url: listing.icon_url,
            published_at: listing.published_at.map(|dt| dt.to_rfc3339()),
            submitted_at: listing.review_submitted_at.map(|dt| dt.to_rfc3339()),
            rejected_at: listing.review_decision_at.map(|dt| dt.to_rfc3339()),
            author_id: listing.author_id,
            author_email,
            rejection_reason,
            rejection_notes,
            created_at: coach.created_at.to_rfc3339(),
            publish_status: listing.publish_status.as_str().to_owned(),
        }
    }
}

/// Response for store coach listing
#[derive(Debug, Serialize)]
pub struct StoreCoachesResponse {
    /// List of coaches
    pub coaches: Vec<StoreCoachResponse>,
    /// Total count
    pub total: u32,
    /// Response metadata
    pub metadata: CoachesMetadata,
}

/// Store action response (approve/reject/unpublish)
#[derive(Debug, Serialize)]
pub struct StoreActionResponse {
    /// Whether the action was successful
    pub success: bool,
    /// Message describing the action
    pub message: String,
    /// Coach ID that was acted upon
    pub coach_id: String,
}

/// Request body for rejecting a coach
#[derive(Debug, Deserialize)]
pub struct RejectCoachBody {
    /// Rejection reason code
    pub reason: String,
    /// Optional additional notes
    pub notes: Option<String>,
}
