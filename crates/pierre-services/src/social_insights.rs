// ABOUTME: Social insights domain service for multi-step social feature orchestration
// ABOUTME: Extracts friend-request validation, feed aggregation, and insight adaptation from routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AdaptedInsight, FriendConnection, FriendStatus, SharedInsight};
use pierre_database::repositories::SocialRepository;
use pierre_intelligence::insight_adapter::{InsightAdapter, UserTrainingContext};
use pierre_llm::{ChatMessage, ChatRequest, LlmProvider};
use serde::Deserialize;
use uuid::Uuid;

/// Temperature for the audience-adaptation rewrite.
///
/// Modest: the rewrite must stay faithful to the source facts while
/// re-voicing for the audience, so we keep creativity low.
const ADAPTATION_TEMPERATURE: f32 = 0.5;

/// Maximum length of the original insight content embedded in the prompt.
///
/// Insights are short social posts; this caps a pathological input so a
/// single adaptation request can't balloon the prompt.
const MAX_PROMPT_INSIGHT_LEN: usize = 2_000;

/// Result of sending a friend request, including validation
pub struct FriendRequestResult {
    /// The created friend connection
    pub connection: FriendConnection,
}

/// A search result enriched with friend-status information
pub struct EnrichedUserResult {
    /// User ID
    pub user_id: Uuid,
    /// Display name if set
    pub display_name: Option<String>,
    /// Email visible only to connected friends
    pub visible_email: Option<String>,
    /// Whether the searcher and this user are connected friends
    pub is_friend: bool,
    /// Whether there is a pending friend request
    pub has_pending_request: bool,
}

/// Result of adapting a shared insight for a user
pub struct AdaptInsightResult {
    /// The adapted insight (newly created or existing)
    pub adapted: AdaptedInsight,
    /// The source insight that was adapted
    pub source_insight: SharedInsight,
    /// Whether this was an existing adaptation (true) or newly created (false)
    pub already_existed: bool,
}

/// Validate and create a friend connection request.
///
/// Enforces business rules:
/// - Cannot send request to yourself
/// - Cannot create duplicate connections
/// - Creates connection in Pending status
///
/// # Errors
///
/// Returns `AppError::InvalidInput` if sender and receiver are the same user,
/// or if a connection already exists between the two users.
/// Returns database errors on connection creation failure.
pub async fn create_friend_request(
    social: &dyn SocialRepository,
    sender_id: Uuid,
    receiver_id: Uuid,
) -> AppResult<FriendRequestResult> {
    // Self-request validation
    if sender_id == receiver_id {
        return Err(AppError::invalid_input(
            "Cannot send friend request to yourself",
        ));
    }

    // Duplicate check
    let existing = social
        .get_friend_connection_between(sender_id, receiver_id)
        .await?;
    if existing.is_some() {
        return Err(AppError::invalid_input(
            "Friend connection already exists between these users",
        ));
    }

    // Create connection
    let connection = FriendConnection::new(sender_id, receiver_id);
    social.create_friend_connection(&connection).await?;

    Ok(FriendRequestResult { connection })
}

/// Search for discoverable users with friend-status enrichment.
///
/// Business rules:
/// - Excludes the searching user from results
/// - Enriches each result with friend connection status
/// - Email is only visible to connected friends (privacy protection)
///
/// # Errors
///
/// Returns database errors on search or connection lookup failure.
pub async fn search_users_with_status(
    social: &dyn SocialRepository,
    searcher_id: Uuid,
    query: &str,
    limit: u32,
) -> AppResult<Vec<EnrichedUserResult>> {
    let users = social
        .search_discoverable_users(query, searcher_id, limit)
        .await?;

    let mut results = Vec::with_capacity(users.len());
    for (user_id, email, display_name) in users {
        // Filter out self
        if user_id == searcher_id {
            continue;
        }

        let connection = social
            .get_friend_connection_between(searcher_id, user_id)
            .await?;

        let is_friend = connection.as_ref().is_some_and(|c| c.status.is_connected());
        let has_pending_request = connection
            .as_ref()
            .is_some_and(|c| c.status == FriendStatus::Pending);

        // Only expose email to connected friends for privacy
        let visible_email = if is_friend { Some(email) } else { None };

        results.push(EnrichedUserResult {
            user_id,
            display_name,
            visible_email,
            is_friend,
            has_pending_request,
        });
    }

    Ok(results)
}

/// Adapt a shared insight for a user's training context.
///
/// Business rules:
/// - Verifies the source insight exists and is visible to the user
/// - Returns existing adaptation if user already adapted this insight (idempotent)
/// - Builds user training context from their activities
/// - Rewrites the insight for the requesting audience with the LLM, preserving
///   every fact/number, when an `llm_provider` is supplied
/// - Falls back to the deterministic `InsightAdapter` template when no provider
///   is wired or the LLM call fails, so a degraded adaptation never blanks the
///   insight
///
/// The `InsightAdapter` template still produces the stored `adaptation_context`
/// (the audience-keying summary) and the guaranteed fallback content, so the
/// cageux path is fallback-only rather than dead.
///
/// # Errors
///
/// Returns `AppError::NotFound` if the source insight does not exist.
/// Returns database errors on adaptation lookup or creation failure.
pub async fn adapt_insight_for_user(
    social: &dyn SocialRepository,
    user_id: Uuid,
    insight_id: Uuid,
    user_context: &UserTrainingContext,
    additional_context: Option<&str>,
    llm_provider: Option<&Arc<dyn LlmProvider>>,
) -> AppResult<AdaptInsightResult> {
    // Verify insight exists and is visible to user
    let source_insight = social
        .get_shared_insight(insight_id, user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Insight {insight_id}")))?;

    // Check for existing adaptation (idempotent)
    if let Some(existing) = social.get_user_adaptation(insight_id, user_id).await? {
        return Ok(AdaptInsightResult {
            adapted: existing,
            source_insight,
            already_existed: true,
        });
    }

    // Deterministic template adaptation: the guaranteed fallback content and the
    // source of the stored audience-keying context summary.
    let insight_adapter = InsightAdapter::new();
    let template_result = insight_adapter.adapt(&source_insight, user_context, additional_context);

    // Try the real LLM rewrite for this audience; fall back to the template
    // content on any failure so a degraded adaptation never blanks the insight.
    let adapted_content = adapt_content_with_llm(
        llm_provider,
        &source_insight,
        user_context,
        additional_context,
        &template_result.adapted_content,
    )
    .await;

    // Build the adapted insight, keeping the template's context summary but
    // swapping in the audience-specific (LLM or fallback) content.
    let mut adapted_insight =
        InsightAdapter::create_adapted_insight(insight_id, user_id, &template_result);
    adapted_insight.adapted_content = adapted_content;

    social.create_adapted_insight(&adapted_insight).await?;

    Ok(AdaptInsightResult {
        adapted: adapted_insight,
        source_insight,
        already_existed: false,
    })
}

/// Rewrite an insight for the requesting audience via the LLM.
///
/// Returns the LLM rewrite on success, or `fallback_content` (the deterministic
/// template adaptation) when no provider is wired, the call fails, or the
/// response is empty/unparseable — a degraded adaptation must never blank the
/// insight.
async fn adapt_content_with_llm(
    llm_provider: Option<&Arc<dyn LlmProvider>>,
    source_insight: &SharedInsight,
    user_context: &UserTrainingContext,
    additional_context: Option<&str>,
    fallback_content: &str,
) -> String {
    let Some(provider) = llm_provider else {
        return fallback_content.to_owned();
    };

    let system_prompt = build_adaptation_prompt(source_insight, user_context, additional_context);
    let messages = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(truncate_for_prompt(&source_insight.content)),
    ];
    let request = ChatRequest::new(messages).with_temperature(ADAPTATION_TEMPERATURE);

    match provider.complete(&request).await {
        Ok(response) => parse_adaptation_response(&response.content, fallback_content),
        Err(error) => {
            tracing::warn!(
                %error,
                insight_id = %source_insight.id,
                "Insight adaptation LLM call failed — falling back to template adaptation"
            );
            fallback_content.to_owned()
        }
    }
}

/// Truncate insight content embedded in the prompt to a sane bound.
fn truncate_for_prompt(content: &str) -> String {
    if content.len() <= MAX_PROMPT_INSIGHT_LEN {
        content.to_owned()
    } else {
        let mut end = MAX_PROMPT_INSIGHT_LEN;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &content[..end])
    }
}

/// Build the system prompt for audience-specific insight adaptation.
///
/// Carries the same audience-keying inputs the deterministic adapter uses —
/// the insight type, the target's fitness level / training phase / weekly
/// volume / primary sport / training goal, and any caller-supplied context —
/// so the rewrite is genuinely audience-specific. The prompt mandates a
/// JSON-only response and that every fact/number from the original be
/// preserved.
///
/// This is a pure function so it can be unit-tested without a live LLM.
#[must_use]
pub fn build_adaptation_prompt(
    source_insight: &SharedInsight,
    user_context: &UserTrainingContext,
    additional_context: Option<&str>,
) -> String {
    let mut audience = Vec::new();
    audience.push(format!("fitness level: {:?}", user_context.fitness_level()));
    if let Some(phase) = user_context.training_phase {
        audience.push(format!("training phase: {phase}"));
    }
    if let Some(hours) = user_context.weekly_volume_hours {
        audience.push(format!("weekly training volume: {hours:.1} hours"));
    }
    if let Some(sport) = &user_context.primary_sport {
        audience.push(format!("primary sport: {sport}"));
    }
    if let Some(goal) = &user_context.training_goal {
        audience.push(format!("training goal: {goal}"));
    }
    audience.push(format!(
        "recent activity count (7d): {}",
        user_context.recent_activity_count
    ));
    audience.push(format!(
        "days since last workout: {}",
        user_context.days_since_last_workout
    ));
    let audience_block = audience.join("\n- ");

    let extra = additional_context
        .map(truncate_for_prompt)
        .map_or_else(String::new, |c| {
            format!("\n\nAdditional context from the requester:\n{c}")
        });

    format!(
        "You are a fitness coach rewriting a friend's shared {insight_type} so it \
speaks directly to a specific reader. Rewrite the insight in the reader's voice \
and for their situation, making it relevant and actionable for them.\n\n\
STRICT RULES:\n\
- Preserve EVERY fact, number, distance, pace, duration, and achievement from the \
original exactly. Never invent, inflate, or drop a number.\n\
- Do not add facts about the reader that are not given below.\n\
- Keep it concise (1-3 short sentences) and encouraging.\n\
- Respond with ONLY a JSON object of the form {{\"adapted_content\": \"...\"}} and \
nothing else.\n\n\
The reader's training profile:\n- {audience_block}{extra}",
        insight_type = source_insight.insight_type.description(),
    )
}

/// Parse the LLM adaptation response.
///
/// Expected format: `{"adapted_content": "..."}`. Tolerates the JSON being
/// wrapped in markdown code fences. Returns `fallback_content` when the
/// response cannot be parsed or yields empty content, so a degraded adaptation
/// never blanks the insight.
///
/// This is a pure function so it can be unit-tested without a live LLM.
#[must_use]
pub fn parse_adaptation_response(raw_content: &str, fallback_content: &str) -> String {
    #[derive(Deserialize)]
    struct AdaptationResponse {
        adapted_content: String,
    }

    let parse = |s: &str| -> Option<AdaptationResponse> { serde_json::from_str(s).ok() };

    let parsed = parse(raw_content).or_else(|| {
        let trimmed = raw_content.trim();
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        if start <= end {
            parse(&trimmed[start..=end])
        } else {
            None
        }
    });

    match parsed {
        Some(response) if !response.adapted_content.trim().is_empty() => {
            response.adapted_content.trim().to_owned()
        }
        _ => {
            tracing::warn!("Failed to parse insight adaptation response — using template fallback");
            fallback_content.to_owned()
        }
    }
}
