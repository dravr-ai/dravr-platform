// ABOUTME: Data transfer types for seeder binaries that populate reference and demo data
// ABOUTME: Used by SeederRepository trait implementations for SQLite and PostgreSQL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Seed-specific data types used by the [`SeederRepository`](crate::repositories::SeederRepository) trait.
//!
//! These types carry owned data from seeder binaries into database-backend-specific
//! implementations, avoiding lifetime complexity across async boundaries.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use pierre_core::models::TenantId;

// ================================
// LLM Usage Seeder
// ================================

/// LLM usage record for seeding analytics dashboards
pub struct SeedLlmUsageRecord {
    /// Unique record ID
    pub id: Uuid,
    /// Tenant that owns this usage
    pub tenant_id: Uuid,
    /// User who triggered the call
    pub user_id: Uuid,
    /// Associated conversation (if from chat)
    pub conversation_id: Option<String>,
    /// LLM provider name (e.g., "gemini", "groq")
    pub provider: String,
    /// Model identifier (e.g., "gemini-2.5-flash-preview")
    pub model: String,
    /// Tokens in the prompt
    pub prompt_tokens: i64,
    /// Tokens in the completion
    pub completion_tokens: i64,
    /// Total tokens (prompt + completion)
    pub total_tokens: i64,
    /// Call type (e.g., "chat", "insight", "`mcp_tool`")
    pub call_type: String,
    /// Number of tool calls made
    pub tool_calls_count: i64,
    /// Execution time in milliseconds
    pub execution_time_ms: i64,
    /// When the call was made
    pub created_at: DateTime<Utc>,
}

// ================================
// Synthetic Activities Seeder
// ================================

/// Synthetic activity record for testing without OAuth providers
pub struct SeedSyntheticActivity {
    /// Unique activity ID
    pub id: Uuid,
    /// User who "did" the activity
    pub user_id: Uuid,
    /// Tenant scope
    pub tenant_id: Uuid,
    /// Activity name (e.g., "Morning Run #42")
    pub name: String,
    /// Sport type (e.g., "run", "ride", "`nordic_ski`")
    pub sport_type: String,
    /// When the activity started
    pub start_date: DateTime<Utc>,
    /// Duration in seconds
    pub duration_seconds: i64,
    /// Distance in meters (None for non-distance activities)
    pub distance_meters: Option<f64>,
    /// Elevation gain in meters
    pub elevation_gain: Option<f64>,
    /// Average heart rate in BPM
    pub average_heart_rate: i32,
    /// Maximum heart rate in BPM
    pub max_heart_rate: i32,
    /// Average speed in m/s
    pub average_speed: Option<f64>,
    /// Maximum speed in m/s
    pub max_speed: Option<f64>,
    /// Estimated calories burned
    pub calories: Option<i32>,
    /// City where activity took place
    pub city: String,
    /// Region/state/province
    pub region: String,
    /// Country
    pub country: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Provider connection for registering synthetic data source
pub struct SeedProviderConnection {
    /// Unique connection ID
    pub id: Uuid,
    /// User who owns the connection
    pub user_id: Uuid,
    /// Tenant scope
    pub tenant_id: Uuid,
    /// Provider name (e.g., "synthetic")
    pub provider: String,
    /// Connection type (e.g., "synthetic")
    pub connection_type: String,
    /// When the connection was established
    pub connected_at: DateTime<Utc>,
    /// JSON metadata about the connection
    pub metadata: String,
}

// ================================
// Social Data Seeder
// ================================

/// User social settings for seeding
pub struct SeedSocialSettings {
    /// User ID
    pub user_id: Uuid,
    /// Whether the user is discoverable by others
    pub discoverable: bool,
    /// Default visibility for shared content (`friends_only` or `public`)
    pub default_visibility: String,
    /// JSON array of activity types to share
    pub share_activity_types: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Friend connection between two users
pub struct SeedFriendConnection {
    /// Unique connection ID
    pub id: Uuid,
    /// User who sent the friend request
    pub initiator_id: Uuid,
    /// User who received the friend request
    pub receiver_id: Uuid,
    /// Connection status ("accepted", "pending", "declined")
    pub status: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
    /// When the connection was accepted (None if pending/declined)
    pub accepted_at: Option<DateTime<Utc>>,
}

/// Shared insight content for social feed
pub struct SeedSharedInsight {
    /// Unique insight ID
    pub id: Uuid,
    /// User who shared the insight
    pub user_id: Uuid,
    /// Visibility level (`friends_only` or `public`)
    pub visibility: String,
    /// Type of insight ("achievement", "milestone", "`training_tip`", etc.)
    pub insight_type: String,
    /// Sport type (None for general insights)
    pub sport_type: Option<String>,
    /// Full insight content
    pub content: String,
    /// Short title
    pub title: String,
    /// Training phase context (None for general)
    pub training_phase: Option<String>,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Reaction to a shared insight
pub struct SeedInsightReaction {
    /// Unique reaction ID
    pub id: Uuid,
    /// Insight being reacted to
    pub insight_id: Uuid,
    /// User who reacted
    pub user_id: Uuid,
    /// Reaction type ("like", "celebrate", "inspire", "support")
    pub reaction_type: String,
    /// When the reaction was created
    pub created_at: DateTime<Utc>,
}

/// Adapted insight from another user's shared content
pub struct SeedAdaptedInsight {
    /// Unique adapted insight ID
    pub id: Uuid,
    /// Source insight that was adapted
    pub source_insight_id: Uuid,
    /// User who adapted the insight
    pub user_id: Uuid,
    /// Adapted content text
    pub adapted_content: String,
    /// JSON context for the adaptation
    pub adaptation_context: String,
    /// Whether the user found the adaptation helpful
    pub was_helpful: bool,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
}

// ================================
// Demo Data Seeder
// ================================

/// Demo user for seeding test data
pub struct SeedDemoUser {
    /// User ID
    pub id: Uuid,
    /// Email address
    pub email: String,
    /// Display name
    pub display_name: String,
    /// Bcrypt-hashed password
    pub password_hash: String,
    /// User tier ("starter", "professional", "enterprise")
    pub tier: String,
    /// Account status ("active", "pending", "suspended")
    pub status: String,
    /// Whether this user has admin privileges
    pub is_admin: bool,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
}

/// Tenant for seeding
pub struct SeedTenant {
    /// Tenant ID
    pub id: Uuid,
    /// Tenant display name
    pub name: String,
    /// URL-safe slug
    pub slug: String,
    /// Plan tier ("starter", "professional", "enterprise")
    pub plan: String,
    /// User who owns this tenant
    pub owner_user_id: Uuid,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// API key for seeding
pub struct SeedApiKey {
    /// Key ID
    pub id: Uuid,
    /// User who owns the key
    pub user_id: Uuid,
    /// Key name
    pub name: String,
    /// Key description
    pub description: String,
    /// Hashed key value
    pub key_hash: String,
    /// Key prefix for identification
    pub key_prefix: String,
    /// Key tier
    pub tier: String,
    /// Rate limit (requests per window)
    pub rate_limit: Option<i32>,
    /// When the key expires (None = no expiry)
    pub expires_at: Option<DateTime<Utc>>,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
}

/// A2A client for seeding
pub struct SeedA2AClient {
    /// Client ID
    pub id: Uuid,
    /// User who owns the client
    pub user_id: Uuid,
    /// Client name
    pub name: String,
    /// Client description
    pub description: String,
    /// Public key
    pub public_key: String,
    /// Client secret (hashed)
    pub client_secret: String,
    /// JSON permissions array
    pub permissions: String,
    /// JSON capabilities array
    pub capabilities: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// API key usage record for seeding analytics
pub struct SeedApiKeyUsage {
    /// Record ID
    pub id: Uuid,
    /// API key that was used
    pub api_key_id: Uuid,
    /// When the call was made
    pub timestamp: DateTime<Utc>,
    /// Tool that was called
    pub tool_name: String,
    /// HTTP status code returned
    pub status_code: i32,
    /// Response time in milliseconds
    pub response_time_ms: i32,
}

/// A2A usage record for seeding analytics
pub struct SeedA2AUsage {
    /// Record ID
    pub id: Uuid,
    /// A2A client that made the call
    pub client_id: Uuid,
    /// When the call was made
    pub timestamp: DateTime<Utc>,
    /// Tool that was called
    pub tool_name: String,
    /// HTTP status code returned
    pub status_code: i32,
    /// Response time in milliseconds
    pub response_time_ms: i32,
}

// ================================
// Coach Seeder
// ================================

/// Coach record for seeding (flat database columns, not the parsed markdown structure)
pub struct SeedCoach {
    /// Coach ID
    pub id: String,
    /// Admin user who created the coach
    pub user_id: Uuid,
    /// Tenant scope
    pub tenant_id: TenantId,
    /// Display title
    pub title: String,
    /// Short description (same as purpose)
    pub description: String,
    /// System prompt (same as instructions)
    pub system_prompt: String,
    /// Coach category
    pub category: String,
    /// JSON array of tags
    pub tags_json: String,
    /// JSON array of sample prompts
    pub sample_prompts_json: String,
    /// Estimated token count
    pub token_count: i64,
    /// Visibility level
    pub visibility: String,
    /// Unique slug identifier
    pub slug: String,
    /// Purpose section text
    pub purpose: Option<String>,
    /// When-to-use section text
    pub when_to_use: Option<String>,
    /// Instructions section text
    pub instructions: Option<String>,
    /// Example inputs section text
    pub example_inputs: Option<String>,
    /// Example outputs section text
    pub example_outputs: Option<String>,
    /// Success criteria section text
    pub success_criteria: Option<String>,
    /// JSON array of prerequisites
    pub prerequisites_json: String,
    /// Source markdown file path
    pub source_file: Option<String>,
    /// Content hash for change detection
    pub content_hash: Option<String>,
    /// Startup query text
    pub startup_query: Option<String>,
    /// JSON-serialized data requirements for deterministic pre-fetching
    pub data_requirements: Option<String>,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Coach relation for seeding
pub struct SeedCoachRelation {
    /// Relation ID
    pub id: String,
    /// Source coach ID
    pub coach_id: String,
    /// Related coach ID
    pub related_coach_id: String,
    /// Relation type ("related", "alternative", "prerequisite", "sequel")
    pub relation_type: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
}

/// Coach author profile for seeding (Store creator profiles)
pub struct SeedCoachAuthor {
    /// Author profile ID (TEXT PK in `coach_authors`)
    pub id: String,
    /// User who is the author
    pub user_id: Uuid,
    /// Tenant scope
    pub tenant_id: String,
    /// Display name in Store
    pub display_name: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
    /// Record update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Store listing for seeding
pub struct SeedStoreListing {
    /// Listing ID
    pub id: String,
    /// Coach being listed
    pub coach_id: String,
    /// Tenant scope
    pub tenant_id: TenantId,
    /// Coach author ID (references `coach_authors.id`, not `users.id`)
    pub author_id: String,
    /// Record creation timestamp
    pub created_at: DateTime<Utc>,
}
