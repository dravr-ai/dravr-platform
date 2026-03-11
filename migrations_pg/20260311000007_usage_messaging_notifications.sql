-- ABOUTME: Consolidated PostgreSQL migration for usage tracking, messaging gateway, and notification tables.
-- ABOUTME: Creates LLM usage, tool catalog, messaging channels/sessions/queue, and push notification tables.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- llm_usage
-- ============================================================================
CREATE TABLE IF NOT EXISTS llm_usage (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    conversation_id VARCHAR(255),
    provider VARCHAR(255) NOT NULL,
    model VARCHAR(255) NOT NULL,
    prompt_tokens BIGINT NOT NULL,
    completion_tokens BIGINT NOT NULL,
    total_tokens BIGINT NOT NULL,
    call_type VARCHAR(50) NOT NULL,
    tool_calls_count BIGINT DEFAULT 0,
    execution_time_ms BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_llm_usage_tenant_user_date ON llm_usage(tenant_id, user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_llm_usage_tenant_date ON llm_usage(tenant_id, created_at);

-- ============================================================================
-- usage_counters
-- ============================================================================
CREATE TABLE IF NOT EXISTS usage_counters (
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    counter_key VARCHAR(255) NOT NULL,
    period VARCHAR(50) NOT NULL,
    value BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, user_id, counter_key, period)
);
CREATE INDEX IF NOT EXISTS idx_usage_counters_tenant_user ON usage_counters(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_usage_counters_period ON usage_counters(period);

-- ============================================================================
-- tool_catalog
-- ============================================================================
CREATE TABLE IF NOT EXISTS tool_catalog (
    id VARCHAR(255) PRIMARY KEY,
    tool_name VARCHAR(255) NOT NULL UNIQUE,
    display_name VARCHAR(255) NOT NULL,
    description TEXT NOT NULL,
    category VARCHAR(50) NOT NULL CHECK (category IN (
        'fitness', 'analysis', 'goals', 'nutrition',
        'recipes', 'sleep', 'configuration', 'connections'
    )),
    is_enabled_by_default BOOLEAN NOT NULL DEFAULT TRUE,
    requires_provider VARCHAR(50),
    min_plan VARCHAR(50) NOT NULL DEFAULT 'starter' CHECK (min_plan IN ('starter', 'professional', 'enterprise')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_tool_catalog_category ON tool_catalog(category);
CREATE INDEX IF NOT EXISTS idx_tool_catalog_min_plan ON tool_catalog(min_plan);

-- ============================================================================
-- tenant_tool_overrides
-- ============================================================================
CREATE TABLE IF NOT EXISTS tenant_tool_overrides (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    tool_name VARCHAR(255) NOT NULL REFERENCES tool_catalog(tool_name) ON DELETE CASCADE,
    is_enabled BOOLEAN NOT NULL,
    enabled_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, tool_name)
);
CREATE INDEX IF NOT EXISTS idx_tenant_tool_overrides_tenant ON tenant_tool_overrides(tenant_id);

-- ============================================================================
-- Seed tool_catalog with all 47 tools
-- ============================================================================
INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
    ('tc-001', 'get_activities', 'Get Activities', 'Get user fitness activities with optional filtering and limits', 'fitness', TRUE, NULL, 'starter'),
    ('tc-002', 'get_athlete', 'Get Athlete Profile', 'Get user athlete profile and basic information', 'fitness', TRUE, NULL, 'starter'),
    ('tc-003', 'get_stats', 'Get Statistics', 'Get user performance statistics and metrics', 'fitness', TRUE, NULL, 'starter'),
    ('tc-004', 'analyze_activity', 'Analyze Activity', 'Analyze a specific activity with detailed performance insights', 'analysis', TRUE, NULL, 'starter'),
    ('tc-005', 'get_activity_intelligence', 'Activity Intelligence', 'Get AI-powered intelligence analysis for an activity', 'analysis', TRUE, NULL, 'starter'),
    ('tc-006', 'get_connection_status', 'Connection Status', 'Check OAuth connection status for fitness providers', 'connections', TRUE, NULL, 'starter'),
    ('tc-007', 'connect_provider', 'Connect Provider', 'Connect to a fitness data provider via OAuth', 'connections', TRUE, NULL, 'starter'),
    ('tc-008', 'disconnect_provider', 'Disconnect Provider', 'Disconnect user from a fitness data provider', 'connections', TRUE, NULL, 'starter'),
    ('tc-009', 'set_goal', 'Set Goal', 'Set a new fitness goal for the user', 'goals', TRUE, NULL, 'starter'),
    ('tc-010', 'suggest_goals', 'Suggest Goals', 'Get AI-suggested fitness goals based on activity history', 'goals', TRUE, NULL, 'starter'),
    ('tc-011', 'analyze_goal_feasibility', 'Goal Feasibility', 'Analyze whether a goal is achievable given current fitness level', 'goals', TRUE, NULL, 'professional'),
    ('tc-012', 'track_progress', 'Track Progress', 'Track progress towards fitness goals', 'goals', TRUE, NULL, 'starter'),
    ('tc-013', 'calculate_metrics', 'Calculate Metrics', 'Calculate custom fitness metrics and performance indicators', 'analysis', TRUE, NULL, 'starter'),
    ('tc-014', 'analyze_performance_trends', 'Performance Trends', 'Analyze performance trends over time', 'analysis', TRUE, NULL, 'professional'),
    ('tc-015', 'compare_activities', 'Compare Activities', 'Compare two activities for performance analysis', 'analysis', TRUE, NULL, 'starter'),
    ('tc-016', 'detect_patterns', 'Detect Patterns', 'Detect patterns and insights in activity data', 'analysis', TRUE, NULL, 'professional'),
    ('tc-017', 'generate_recommendations', 'Generate Recommendations', 'Generate personalized training recommendations', 'analysis', TRUE, NULL, 'professional'),
    ('tc-018', 'calculate_fitness_score', 'Fitness Score', 'Calculate overall fitness score based on recent activities', 'analysis', TRUE, NULL, 'starter'),
    ('tc-019', 'predict_performance', 'Predict Performance', 'Predict future performance based on training patterns', 'analysis', TRUE, NULL, 'enterprise'),
    ('tc-020', 'analyze_training_load', 'Training Load', 'Analyze training load and recovery metrics', 'analysis', TRUE, NULL, 'professional'),
    ('tc-021', 'get_configuration_catalog', 'Configuration Catalog', 'Get the complete configuration catalog with all available parameters', 'configuration', TRUE, NULL, 'starter'),
    ('tc-022', 'get_configuration_profiles', 'Configuration Profiles', 'Get available configuration profiles (Research, Elite, Recreational, etc.)', 'configuration', TRUE, NULL, 'starter'),
    ('tc-023', 'get_user_configuration', 'Get User Config', 'Get current user configuration settings and overrides', 'configuration', TRUE, NULL, 'starter'),
    ('tc-024', 'update_user_configuration', 'Update User Config', 'Update user configuration parameters and session overrides', 'configuration', TRUE, NULL, 'starter'),
    ('tc-025', 'calculate_personalized_zones', 'Personalized Zones', 'Calculate personalized training zones based on user VO2 max', 'configuration', TRUE, NULL, 'starter'),
    ('tc-026', 'validate_configuration', 'Validate Config', 'Validate configuration parameters against safety rules', 'configuration', TRUE, NULL, 'starter'),
    ('tc-027', 'analyze_sleep_quality', 'Sleep Quality', 'Analyze sleep quality from Fitbit/Garmin data using NSF/AASM guidelines', 'sleep', TRUE, NULL, 'professional'),
    ('tc-028', 'calculate_recovery_score', 'Recovery Score', 'Calculate holistic recovery score combining TSB, sleep quality, and HRV', 'sleep', TRUE, NULL, 'professional'),
    ('tc-029', 'suggest_rest_day', 'Rest Day Suggestion', 'AI-powered rest day recommendation based on recovery indicators', 'sleep', TRUE, NULL, 'professional'),
    ('tc-030', 'track_sleep_trends', 'Sleep Trends', 'Track sleep patterns and correlate with performance over time', 'sleep', TRUE, NULL, 'professional'),
    ('tc-031', 'optimize_sleep_schedule', 'Sleep Schedule', 'Optimize sleep duration based on training load and recovery needs', 'sleep', TRUE, NULL, 'enterprise'),
    ('tc-032', 'get_fitness_config', 'Get Fitness Config', 'Get user fitness configuration settings including heart rate zones', 'configuration', TRUE, NULL, 'starter'),
    ('tc-033', 'set_fitness_config', 'Set Fitness Config', 'Save user fitness configuration settings for zones and thresholds', 'configuration', TRUE, NULL, 'starter'),
    ('tc-034', 'list_fitness_configs', 'List Fitness Configs', 'List all available fitness configuration names for the user', 'configuration', TRUE, NULL, 'starter'),
    ('tc-035', 'delete_fitness_config', 'Delete Fitness Config', 'Delete a specific fitness configuration by name', 'configuration', TRUE, NULL, 'starter'),
    ('tc-036', 'calculate_daily_nutrition', 'Daily Nutrition', 'Calculate daily calorie and macronutrient needs using Mifflin-St Jeor BMR formula', 'nutrition', TRUE, NULL, 'starter'),
    ('tc-037', 'get_nutrient_timing', 'Nutrient Timing', 'Get optimal pre/post-workout nutrition recommendations following ISSN guidelines', 'nutrition', TRUE, NULL, 'professional'),
    ('tc-038', 'search_food', 'Search Food', 'Search USDA FoodData Central database for foods by name/description', 'nutrition', TRUE, NULL, 'starter'),
    ('tc-039', 'get_food_details', 'Food Details', 'Get detailed nutritional information for a specific food from USDA database', 'nutrition', TRUE, NULL, 'starter'),
    ('tc-040', 'analyze_meal_nutrition', 'Meal Nutrition', 'Analyze total calories and macronutrients for a meal of multiple foods', 'nutrition', TRUE, NULL, 'starter'),
    ('tc-041', 'get_recipe_constraints', 'Recipe Constraints', 'Get macro targets for LLM recipe generation by training phase', 'recipes', TRUE, NULL, 'starter'),
    ('tc-042', 'validate_recipe', 'Validate Recipe', 'Validate recipe nutrition against USDA and calculate macros', 'recipes', TRUE, NULL, 'starter'),
    ('tc-043', 'save_recipe', 'Save Recipe', 'Save validated recipe with cached nutrition data', 'recipes', TRUE, NULL, 'starter'),
    ('tc-044', 'list_recipes', 'List Recipes', 'List saved recipes with optional meal timing filter', 'recipes', TRUE, NULL, 'starter'),
    ('tc-045', 'get_recipe', 'Get Recipe', 'Get a specific recipe by ID', 'recipes', TRUE, NULL, 'starter'),
    ('tc-046', 'delete_recipe', 'Delete Recipe', 'Delete a recipe from collection', 'recipes', TRUE, NULL, 'starter'),
    ('tc-047', 'search_recipes', 'Search Recipes', 'Search recipes by name, tags, or description', 'recipes', TRUE, NULL, 'starter')
ON CONFLICT (tool_name) DO NOTHING;

-- ============================================================================
-- messaging_channel_configs
-- ============================================================================
-- Per-tenant channel configuration for multi-channel chat (WhatsApp, Messenger, etc.)
CREATE TABLE IF NOT EXISTS messaging_channel_configs (
    id TEXT NOT NULL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    api_key TEXT,
    api_secret TEXT,
    webhook_secret TEXT,
    account_id TEXT,
    phone_number TEXT,
    bot_token TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, channel_type)
);
CREATE INDEX IF NOT EXISTS idx_messaging_channel_configs_tenant ON messaging_channel_configs(tenant_id);
CREATE INDEX IF NOT EXISTS idx_messaging_channel_configs_active ON messaging_channel_configs(tenant_id, is_active) WHERE is_active = TRUE;

-- ============================================================================
-- messaging_sessions
-- ============================================================================
-- Active messaging sessions linking channel users to Pierre conversations.
CREATE TABLE IF NOT EXISTS messaging_sessions (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    channel_user_id TEXT NOT NULL,
    channel_conversation_id TEXT,
    pierre_conversation_id TEXT,
    last_message_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_messaging_sessions_tenant ON messaging_sessions(tenant_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_sessions_channel_identity ON messaging_sessions(tenant_id, channel_type, channel_user_id);
CREATE INDEX IF NOT EXISTS idx_messaging_sessions_user ON messaging_sessions(user_id, tenant_id);

-- ============================================================================
-- messaging_messages
-- ============================================================================
-- Message log with idempotency key (channel_message_id per tenant).
CREATE TABLE IF NOT EXISTS messaging_messages (
    id TEXT NOT NULL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES messaging_sessions(id),
    direction TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    channel_message_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    content_type TEXT NOT NULL,
    content_body TEXT,
    correlation_id TEXT NOT NULL,
    raw_payload TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_messaging_messages_tenant ON messaging_messages(tenant_id);
CREATE INDEX IF NOT EXISTS idx_messaging_messages_session ON messaging_messages(session_id, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_messages_idempotency ON messaging_messages(tenant_id, channel_message_id);
CREATE INDEX IF NOT EXISTS idx_messaging_messages_correlation ON messaging_messages(correlation_id);

-- ============================================================================
-- messaging_delivery_receipts
-- ============================================================================
-- Delivery receipts tracking outbound message lifecycle.
CREATE TABLE IF NOT EXISTS messaging_delivery_receipts (
    id TEXT NOT NULL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES messaging_messages(id),
    channel_message_id TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_messaging_delivery_receipts_message ON messaging_delivery_receipts(message_id);
CREATE INDEX IF NOT EXISTS idx_messaging_delivery_receipts_tenant ON messaging_delivery_receipts(tenant_id);

-- ============================================================================
-- messaging_outbound_queue
-- ============================================================================
-- Outbound message queue for retry tracking with dead-letter support.
CREATE TABLE IF NOT EXISTS messaging_outbound_queue (
    id TEXT NOT NULL PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messaging_messages(id),
    tenant_id TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_tenant ON messaging_outbound_queue(tenant_id);
CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_retry ON messaging_outbound_queue(status, next_retry_at) WHERE status = 'pending' OR status LIKE 'retrying:%';
CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_message ON messaging_outbound_queue(message_id);

-- ============================================================================
-- messaging_link_states
-- ============================================================================
-- Ephemeral link states for pending channel linking requests (10-minute TTL, one-time use).
-- Final state: nullable user_id for webhook-initiated flows, with channel identity columns.
CREATE TABLE IF NOT EXISTS messaging_link_states (
    id TEXT NOT NULL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT,
    channel_type TEXT NOT NULL,
    code TEXT NOT NULL,
    method TEXT NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE,
    channel_user_id TEXT,
    sender_name TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_link_states_code ON messaging_link_states(code);
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_expiry ON messaging_link_states(expires_at) WHERE used = FALSE;
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_user_channel ON messaging_link_states(tenant_id, user_id, channel_type) WHERE used = FALSE AND user_id IS NOT NULL;

-- ============================================================================
-- messaging_channel_links
-- ============================================================================
-- Permanent user-to-channel identity mappings.
CREATE TABLE IF NOT EXISTS messaging_channel_links (
    id TEXT NOT NULL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    channel_user_id TEXT NOT NULL,
    display_name TEXT,
    linked_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, channel_type, channel_user_id)
);
CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_user ON messaging_channel_links(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_channel_identity ON messaging_channel_links(tenant_id, channel_type, channel_user_id);

-- ============================================================================
-- oauth_notifications
-- ============================================================================
CREATE TABLE IF NOT EXISTS oauth_notifications (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL,
    provider TEXT NOT NULL,
    success BOOLEAN NOT NULL DEFAULT TRUE,
    message TEXT NOT NULL,
    expires_at TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    read_at TIMESTAMPTZ,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_oauth_notifications_user_id ON oauth_notifications(user_id);
CREATE INDEX IF NOT EXISTS idx_oauth_notifications_user_unread ON oauth_notifications(user_id, read_at) WHERE read_at IS NULL;

-- ============================================================================
-- device_tokens
-- ============================================================================
CREATE TABLE IF NOT EXISTS device_tokens (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    expo_push_token TEXT NOT NULL,
    platform TEXT NOT NULL CHECK (platform IN ('ios', 'android')),
    device_name TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, tenant_id, expo_push_token)
);
CREATE INDEX IF NOT EXISTS idx_device_tokens_user_tenant ON device_tokens(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_device_tokens_tenant ON device_tokens(tenant_id);
CREATE INDEX IF NOT EXISTS idx_device_tokens_active ON device_tokens(user_id, tenant_id, active) WHERE active = TRUE;

-- ============================================================================
-- notification_preferences
-- ============================================================================
CREATE TABLE IF NOT EXISTS notification_preferences (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    category TEXT NOT NULL CHECK (category IN ('training', 'recovery', 'social', 'coach', 'achievement', 'system', 'ai', 'reminders')),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    sub_preferences TEXT,
    quiet_hours_start TEXT,
    quiet_hours_end TEXT,
    timezone TEXT,
    max_per_day INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, tenant_id, category)
);
CREATE INDEX IF NOT EXISTS idx_notification_preferences_user_tenant ON notification_preferences(user_id, tenant_id);

-- ============================================================================
-- notifications
-- ============================================================================
CREATE TABLE IF NOT EXISTS notifications (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    category TEXT NOT NULL,
    notification_type TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    data TEXT,
    image_url TEXT,
    read_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    opened_at TIMESTAMPTZ,
    dismissed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant ON notifications(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant_created ON notifications(user_id, tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant_category ON notifications(user_id, tenant_id, category);
CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant_unread ON notifications(user_id, tenant_id, read_at) WHERE read_at IS NULL;

-- ============================================================================
-- scheduled_notifications
-- ============================================================================
CREATE TABLE IF NOT EXISTS scheduled_notifications (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    notification_type TEXT NOT NULL,
    schedule_cron TEXT NOT NULL,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    next_fire_at TIMESTAMPTZ,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_fired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_scheduled_notifications_user_tenant ON scheduled_notifications(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_scheduled_notifications_next_fire ON scheduled_notifications(next_fire_at, enabled) WHERE enabled = TRUE;
