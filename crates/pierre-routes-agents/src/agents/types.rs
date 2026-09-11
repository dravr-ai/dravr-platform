// ABOUTME: Request and response types for Coaches REST API endpoints
// ABOUTME: Contains all serializable structs for agent CRUD, versioning, admin, and store operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::tool_execution::{MAX_MAX_TOOL_ITERATIONS, MIN_MAX_TOOL_ITERATIONS};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::field_update::FieldUpdate;
use pierre_core::models::agents::DataRequirements;
use pierre_database::database::{
    agents::{
        Agent, AgentAssignment as DbAgentAssignment, AgentCategory, AgentListItem, AgentVersion,
        AgentVisibility, CreateAgentRequest,
        CreateSystemAgentRequest as DbCreateSystemAgentRequest, UpdateAgentRequest,
    },
    store_listings::AgentWithListing,
};
use pierre_services::agent_package::PackageReview;
use serde::{Deserialize, Serialize};

// ============================================
// Core Agent Response Types
// ============================================

/// Response for an agent
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentResponse {
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
    /// Whether this is a system agent (admin-created)
    pub is_system: bool,
    /// Visibility level
    pub visibility: String,
    /// Whether this agent is assigned to the current user
    pub is_assigned: bool,
    /// ID of the agent this was forked from (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Addressable catalogue handle (`@handle`); absent on a personal agent
    /// that was never published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
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
    /// Agent purpose (from ## Purpose section)
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
    /// Personalized relevance score in `0.0..=1.0` (only present when
    /// `personalize=true`). Higher means a better fit for the user's recent
    /// sport mix and connected providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_score: Option<f32>,
    /// Whether this agent is in the user's "Recommended for you" set (only
    /// present when `personalize=true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended: Option<bool>,
    /// Per-agent tool-loop iteration budget for a chat turn. Absent when the
    /// agent inherits the admin configuration value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<i32>,
}

/// A missing prerequisite for an agent
#[derive(Debug, Serialize, Deserialize)]
pub struct MissingPrerequisite {
    /// Type of prerequisite (provider, `activity_count`, `activity_type`)
    pub prerequisite_type: String,
    /// The specific requirement (e.g., "strava", "50 activities", "Run")
    pub requirement: String,
    /// Human-readable message explaining what's missing
    pub message: String,
}

impl From<Agent> for AgentResponse {
    fn from(agent: Agent) -> Self {
        Self {
            id: agent.id.to_string(),
            title: agent.title,
            description: agent.description,
            system_prompt: agent.system_prompt,
            category: agent.category.as_str().to_owned(),
            tags: agent.tags,
            token_count: agent.token_count,
            is_favorite: false, // Defaults; preferences live in coach_assignments
            use_count: 0,
            last_used_at: None,
            created_at: agent.created_at.to_rfc3339(),
            updated_at: agent.updated_at.to_rfc3339(),
            is_system: agent.is_system,
            visibility: agent.visibility.as_str().to_owned(),
            is_assigned: false, // Default for single agent responses
            forked_from: agent.forked_from.map(|id| id.to_string()),
            handle: agent.handle,
            prerequisites_met: None,
            missing_prerequisites: None,
            startup_query: agent.startup_query,
            data_requirements: agent.data_requirements,
            purpose: agent.purpose,
            when_to_use: agent.when_to_use,
            instructions: agent.instructions,
            example_inputs: agent.example_inputs,
            example_outputs: agent.example_outputs,
            success_criteria: agent.success_criteria,
            match_score: None,
            recommended: None,
            max_tool_iterations: agent.max_tool_iterations,
        }
    }
}

impl From<AgentListItem> for AgentResponse {
    fn from(item: AgentListItem) -> Self {
        Self {
            id: item.agent.id.to_string(),
            title: item.agent.title,
            description: item.agent.description,
            system_prompt: item.agent.system_prompt,
            category: item.agent.category.as_str().to_owned(),
            tags: item.agent.tags,
            token_count: item.agent.token_count,
            is_favorite: item.is_favorite,
            use_count: item.use_count,
            last_used_at: item.last_used_at.map(|dt| dt.to_rfc3339()),
            created_at: item.agent.created_at.to_rfc3339(),
            updated_at: item.agent.updated_at.to_rfc3339(),
            is_system: item.agent.is_system,
            visibility: item.agent.visibility.as_str().to_owned(),
            is_assigned: item.is_assigned,
            forked_from: item.agent.forked_from.map(|id| id.to_string()),
            handle: item.agent.handle,
            prerequisites_met: None,
            missing_prerequisites: None,
            startup_query: item.agent.startup_query,
            data_requirements: item.agent.data_requirements,
            purpose: item.agent.purpose,
            when_to_use: item.agent.when_to_use,
            instructions: item.agent.instructions,
            example_inputs: item.agent.example_inputs,
            example_outputs: item.agent.example_outputs,
            success_criteria: item.agent.success_criteria,
            match_score: None,
            recommended: None,
            max_tool_iterations: item.agent.max_tool_iterations,
        }
    }
}

/// Response for listing agents
#[derive(Debug, Serialize, Deserialize)]
pub struct ListAgentsResponse {
    /// List of agents
    pub agents: Vec<AgentResponse>,
    /// Total count of agents matching the filter
    pub total: u32,
    /// Metadata
    pub metadata: AgentsMetadata,
}

/// Metadata for agents response
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentsMetadata {
    /// Response timestamp
    pub timestamp: String,
    /// API version
    pub api_version: String,
}

/// Query parameters for listing agents
#[derive(Debug, Deserialize, Default)]
pub struct ListAgentsQuery {
    /// Filter by category
    pub category: Option<String>,
    /// Filter to favorites only
    pub favorites_only: Option<bool>,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
    /// Include system agents (default: true)
    pub include_system: Option<bool>,
    /// Include hidden agents (default: false)
    pub include_hidden: Option<bool>,
    /// Check prerequisites against user's connected providers (default: false)
    pub check_prerequisites: Option<bool>,
    /// Personalize results: scan the user's recent activities and connected
    /// providers, then mark each agent with a `match_score` and `recommended`
    /// flag (default: false).
    pub personalize: Option<bool>,
}

/// Query parameters for searching agents
#[derive(Debug, Deserialize)]
pub struct SearchAgentsQuery {
    /// Search query string
    pub q: String,
    /// Maximum results to return
    pub limit: Option<u32>,
    /// Pagination offset
    pub offset: Option<u32>,
}

/// Response for toggle favorite
#[derive(Debug, Serialize, Deserialize)]
pub struct ToggleFavoriteResponse {
    /// New favorite status
    pub is_favorite: bool,
}

/// Response for record usage
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordUsageResponse {
    /// Whether the usage was recorded
    pub success: bool,
}

/// Response for hide/show agent operations
#[derive(Debug, Serialize, Deserialize)]
pub struct HideAgentResponse {
    /// Whether the operation was successful
    pub success: bool,
    /// Whether the agent is now hidden (true) or visible (false)
    pub is_hidden: bool,
}

/// Response for forking an agent
#[derive(Debug, Serialize, Deserialize)]
pub struct ForkAgentResponse {
    /// The newly created forked agent
    pub agent: AgentResponse,
    /// The ID of the original agent that was forked
    pub source_agent_id: String,
}

/// Response for importing an agent from markdown
#[derive(Debug, Serialize)]
pub struct ImportAgentResponse {
    /// The created agent
    pub agent: AgentResponse,
    /// The parsed name/slug from the markdown
    pub parsed_name: String,
    /// Estimated token count from the markdown
    pub token_count: u32,
    /// Import warnings (missing optional sections, high token count, etc.)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Response for previewing an agent import without saving
#[derive(Debug, Serialize)]
pub struct ImportPreviewResponse {
    /// Whether the markdown parsed successfully
    pub valid: bool,
    /// Parsed agent fields (present when valid)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedAgentFields>,
    /// Parse errors (present when invalid)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Warnings about missing optional sections or quality issues
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Content hash for deduplication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Whether an agent with this content already exists for the user
    pub duplicate_exists: bool,
    /// ID of the existing duplicate agent (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_agent_id: Option<String>,
    /// Estimated token count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
}

/// Parsed agent fields extracted from markdown for preview
#[derive(Debug, Serialize)]
pub struct ParsedAgentFields {
    /// Agent name/slug from frontmatter
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

/// Request body for importing an agent from a URL
#[derive(Debug, Deserialize)]
pub struct ImportFromUrlBody {
    /// HTTPS URL pointing to a markdown agent definition
    pub url: String,
    /// Whether to save the imported agent (true) or just preview (false)
    #[serde(default = "default_save_true")]
    pub save: bool,
}

const fn default_save_true() -> bool {
    true
}

// ============================================
// Create/Update Request Types
// ============================================

/// Request body for creating an agent (mirrors `CreateAgentRequest` with serde derives)
#[derive(Debug, Deserialize)]
pub struct CreateAgentBody {
    /// Display title for the agent
    pub title: String,
    /// Optional description explaining the agent's purpose
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
    /// Agent purpose (from ## Purpose section)
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
    /// Per-turn tool-loop iteration budget for this agent. Omitted leaves the
    /// agent on the `tool_execution.max_iterations` admin configuration value.
    pub max_tool_iterations: Option<i32>,
}

impl From<CreateAgentBody> for CreateAgentRequest {
    fn from(body: CreateAgentBody) -> Self {
        Self {
            title: body.title,
            description: body.description,
            system_prompt: body.system_prompt,
            category: body
                .category
                .map(|c| AgentCategory::parse(&c))
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
            max_tool_iterations: body.max_tool_iterations,
        }
    }
}

/// Request body for updating an agent
#[derive(Debug, Deserialize)]
pub struct UpdateAgentBody {
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
    /// New per-turn tool-loop iteration budget. An absent key leaves the
    /// stored value untouched; an explicit `null` clears it so the agent
    /// inherits the `tool_execution.max_iterations` admin value again.
    #[serde(default)]
    pub max_tool_iterations: FieldUpdate<i32>,
}

impl From<UpdateAgentBody> for UpdateAgentRequest {
    fn from(body: UpdateAgentBody) -> Self {
        Self {
            title: body.title,
            description: body.description,
            system_prompt: body.system_prompt,
            category: body.category.map(|c| AgentCategory::parse(&c)),
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
            max_tool_iterations: body.max_tool_iterations,
        }
    }
}

/// Reject a tool-loop budget outside the platform band.
///
/// `None` — the key absent, or explicitly cleared back to inherit — always
/// passes; a supplied value must land inside
/// [`MIN_MAX_TOOL_ITERATIONS`] through [`MAX_MAX_TOOL_ITERATIONS`], the same band
/// the chat pipeline reads back and the `tool_execution.max_iterations` admin
/// parameter is bounded to.
///
/// # Errors
///
/// Returns [`ErrorCode::ValueOutOfRange`] naming the band and the value.
pub fn validate_max_tool_iterations(value: Option<i32>) -> Result<(), AppError> {
    let Some(value) = value else {
        return Ok(());
    };
    let min = i32::from(MIN_MAX_TOOL_ITERATIONS);
    let max = i32::from(MAX_MAX_TOOL_ITERATIONS);
    if (min..=max).contains(&value) {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::ValueOutOfRange,
        format!("max_tool_iterations must be between {min} and {max}, got {value}"),
    ))
}

// ============================================
// Version History Response Types
// ============================================

/// Response for an agent version
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentVersionResponse {
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

impl From<AgentVersion> for AgentVersionResponse {
    fn from(v: AgentVersion) -> Self {
        Self {
            version: v.version,
            content_snapshot: v.content_snapshot,
            change_summary: v.change_summary,
            created_at: v.created_at.to_rfc3339(),
            // Left None here; the version route handlers resolve created_by ->
            // display name via resolve_creator_name (async user lookup) since
            // this From impl cannot perform database access.
            created_by_name: None,
        }
    }
}

/// Response for listing agent versions
#[derive(Debug, Serialize, Deserialize)]
pub struct ListVersionsResponse {
    /// List of versions
    pub versions: Vec<AgentVersionResponse>,
    /// Current version number
    pub current_version: i32,
    /// Total number of versions
    pub total: usize,
}

/// Response for reverting to a version
#[derive(Debug, Serialize, Deserialize)]
pub struct RevertVersionResponse {
    /// The agent after reversion
    pub agent: AgentResponse,
    /// The version that was reverted to
    pub reverted_to_version: i32,
    /// The new version number (after revert)
    pub new_version: i32,
}

/// Response for comparing two versions
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentDiffResponse {
    /// Source version number
    pub from_version: i32,
    /// Target version number
    pub to_version: i32,
    /// List of field changes
    pub changes: Vec<FieldChange>,
}

/// A single field change between versions
#[derive(Debug, Serialize, Deserialize)]
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
pub struct ListVersionsQuery {
    /// Maximum number of versions to return
    pub limit: Option<u32>,
}

// ============================================
// Admin Request/Response Types
// ============================================

/// Request body for creating a system agent
#[derive(Debug, Deserialize)]
pub struct AdminCreateAgentBody {
    /// Display title for the agent
    pub title: String,
    /// Optional description explaining the agent's purpose
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

impl From<AdminCreateAgentBody> for DbCreateSystemAgentRequest {
    fn from(body: AdminCreateAgentBody) -> Self {
        Self {
            title: body.title,
            description: body.description,
            system_prompt: body.system_prompt,
            category: body
                .category
                .map(|c| AgentCategory::parse(&c))
                .unwrap_or_default(),
            tags: body.tags,
            sample_prompts: body.sample_prompts,
            visibility: body
                .visibility
                .map_or(AgentVisibility::Tenant, |v| AgentVisibility::parse(&v)),
        }
    }
}

/// Request body for assigning/unassigning agents
#[derive(Debug, Deserialize)]
pub struct AssignAgentBody {
    /// User IDs to assign/unassign
    pub user_ids: Vec<String>,
}

/// Response for agent assignment
#[derive(Debug, Serialize)]
pub struct AssignAgentResponse {
    /// Agent ID
    pub agent_id: String,
    /// Number of users successfully assigned
    pub assigned_count: usize,
    /// Total number of users requested
    pub total_requested: usize,
}

/// Response for agent unassignment
#[derive(Debug, Serialize)]
pub struct UnassignAgentResponse {
    /// Agent ID
    pub agent_id: String,
    /// Number of users successfully unassigned
    pub removed_count: usize,
    /// Total number of users requested
    pub total_requested: usize,
}

/// Agent assignment info
#[derive(Debug, Serialize)]
pub struct AgentAssignment {
    /// User ID
    pub user_id: String,
    /// User email (for display)
    pub user_email: Option<String>,
    /// When assigned
    pub assigned_at: String,
    /// Who assigned
    pub assigned_by: Option<String>,
}

impl From<DbAgentAssignment> for AgentAssignment {
    fn from(db: DbAgentAssignment) -> Self {
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
    /// Agent ID
    pub agent_id: String,
    /// List of assignments
    pub assignments: Vec<AgentAssignment>,
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
    /// Number of agents pending review
    pub pending_count: u32,
    /// Number of published agents
    pub published_count: u32,
    /// Number of rejected agents
    pub rejected_count: u32,
    /// Total installs across all published agents
    pub total_installs: u32,
    /// Rejection rate as percentage
    pub rejection_rate: f64,
}

/// Store agent response with author email
#[derive(Debug, Serialize)]
pub struct StoreAgentResponse {
    /// Agent ID
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
    /// Addressable catalogue handle (`@handle`), assigned at approval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// The training artefacts the agent's package ships — flavour, skeleton,
    /// workouts — each with what in it nothing answers, so the reviewer sees
    /// an unresolved evidence path or an uncited artefact before approving.
    pub package: PackageReview,
}

impl StoreAgentResponse {
    /// Create from `AgentWithListing` with author email and the package review
    pub(super) fn from_agent_with_listing(
        cwl: AgentWithListing,
        author_email: Option<String>,
        package: PackageReview,
    ) -> Self {
        let agent = cwl.agent;
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
            id: agent.id.to_string(),
            title: agent.title,
            description: agent.description,
            system_prompt: agent.system_prompt,
            category: agent.category.as_str().to_owned(),
            tags: agent.tags,
            sample_prompts: agent.sample_prompts,
            token_count: agent.token_count,
            install_count: listing.install_count,
            icon_url: listing.icon_url,
            published_at: listing.published_at.map(|dt| dt.to_rfc3339()),
            submitted_at: listing.review_submitted_at.map(|dt| dt.to_rfc3339()),
            rejected_at: listing.review_decision_at.map(|dt| dt.to_rfc3339()),
            author_id: listing.author_id,
            author_email,
            rejection_reason,
            rejection_notes,
            created_at: agent.created_at.to_rfc3339(),
            publish_status: listing.publish_status.as_str().to_owned(),
            handle: agent.handle,
            package,
        }
    }
}

/// Response for store agent listing
#[derive(Debug, Serialize)]
pub struct StoreAgentsResponse {
    /// List of agents
    pub agents: Vec<StoreAgentResponse>,
    /// Total count
    pub total: u32,
    /// Response metadata
    pub metadata: AgentsMetadata,
}

/// Store action response (approve/reject/unpublish)
#[derive(Debug, Serialize)]
pub struct StoreActionResponse {
    /// Whether the action was successful
    pub success: bool,
    /// Message describing the action
    pub message: String,
    /// Agent ID that was acted upon
    pub agent_id: String,
}

/// Request body for rejecting an agent
#[derive(Debug, Deserialize)]
pub struct RejectAgentBody {
    /// Rejection reason code
    pub reason: String,
    /// Optional additional notes
    pub notes: Option<String>,
}

// ============================================
// Onboarding agent proposal
// ============================================

/// One sport's share of the user's recent activity mix.
#[derive(Debug, Serialize, Deserialize)]
pub struct SportShare {
    /// Canonical `snake_case` sport label (e.g. `run`, `ride`).
    pub sport: String,
    /// Activity count for this sport in the look-back window.
    pub count: u32,
    /// Fraction of total activities this sport represents (`0.0..=1.0`).
    pub share: f32,
}

/// The user's inferred sport profile, rendered on the onboarding
/// "we analyzed your data" screen before agent suggestions.
#[derive(Debug, Serialize, Deserialize)]
pub struct SportProfileSummary {
    /// `false` ⇒ cold start (no connected provider or no activities in the
    /// window); the proposal falls back to broadly-useful starter agents.
    pub has_profile: bool,
    /// Most-logged sport, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_sport: Option<String>,
    /// Total activities scanned to build the profile.
    pub total_activities: u32,
    /// Look-back window (days) the activities were drawn from.
    pub window_days: u32,
    /// Per-sport breakdown, sorted by count descending.
    pub sport_mix: Vec<SportShare>,
}

/// An agent proposed to the user, with its match score and the rationale for
/// surfacing it. `reason` is LLM-authored when the re-ranking step runs, or a
/// deterministic fallback string otherwise.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProposedAgent {
    /// The proposed agent.
    pub agent: AgentResponse,
    /// Relevance score in `0.0..=1.0` from the deterministic prefilter.
    pub match_score: f32,
    /// One-sentence, second-person rationale ("why this coach fits you").
    pub reason: String,
}

/// Response for `GET /api/agents/proposal`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentProposalResponse {
    /// The inferred sport profile shown before the agent list.
    pub profile: SportProfileSummary,
    /// Up to `COACH_REC_MAX_RECOMMENDED` proposed agents, best fit first.
    pub agents: Vec<ProposedAgent>,
    /// Standard response metadata.
    pub metadata: AgentsMetadata,
}
