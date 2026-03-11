-- ABOUTME: Consolidated PostgreSQL migration for content, social, and mobility tables.
-- ABOUTME: Creates chat, recipes, prompts, stretching, yoga, synthetic activities, and social feature tables.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- chat_conversations
-- ============================================================================
CREATE TABLE IF NOT EXISTS chat_conversations (
    id VARCHAR(255) PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id VARCHAR(255) NOT NULL,
    title TEXT NOT NULL,
    model VARCHAR(255) NOT NULL DEFAULT 'gemini-2.0-flash-exp',
    system_prompt TEXT,
    total_tokens BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_user ON chat_conversations(user_id);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_tenant ON chat_conversations(tenant_id);
CREATE INDEX IF NOT EXISTS idx_chat_conversations_updated ON chat_conversations(updated_at DESC);

-- ============================================================================
-- chat_messages
-- ============================================================================
CREATE TABLE IF NOT EXISTS chat_messages (
    id VARCHAR(255) PRIMARY KEY,
    conversation_id VARCHAR(255) NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    role VARCHAR(50) NOT NULL CHECK (role IN ('system', 'user', 'assistant')),
    content TEXT NOT NULL,
    token_count BIGINT,
    prompt_tokens BIGINT,
    model VARCHAR(255),
    finish_reason VARCHAR(50),
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation ON chat_messages(conversation_id);

-- ============================================================================
-- recipes
-- ============================================================================
CREATE TABLE IF NOT EXISTS recipes (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    servings INTEGER NOT NULL DEFAULT 1 CHECK (servings > 0),
    prep_time_mins INTEGER CHECK (prep_time_mins >= 0),
    cook_time_mins INTEGER CHECK (cook_time_mins >= 0),
    instructions TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    meal_timing TEXT NOT NULL DEFAULT 'general' CHECK (meal_timing IN ('pre_training', 'post_training', 'rest_day', 'general')),
    cached_calories DOUBLE PRECISION,
    cached_protein_g DOUBLE PRECISION,
    cached_carbs_g DOUBLE PRECISION,
    cached_fat_g DOUBLE PRECISION,
    cached_fiber_g DOUBLE PRECISION,
    cached_sodium_mg DOUBLE PRECISION,
    cached_sugar_g DOUBLE PRECISION,
    nutrition_validated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recipes_user ON recipes(user_id);
CREATE INDEX IF NOT EXISTS idx_recipes_tenant ON recipes(tenant_id);
CREATE INDEX IF NOT EXISTS idx_recipes_user_tenant ON recipes(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_recipes_meal_timing ON recipes(meal_timing);
CREATE INDEX IF NOT EXISTS idx_recipes_updated ON recipes(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_recipes_created ON recipes(created_at DESC);

-- ============================================================================
-- recipe_ingredients
-- ============================================================================
CREATE TABLE IF NOT EXISTS recipe_ingredients (
    id TEXT PRIMARY KEY,
    recipe_id TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    fdc_id INTEGER,
    name TEXT NOT NULL,
    amount DOUBLE PRECISION NOT NULL CHECK (amount > 0),
    unit TEXT NOT NULL DEFAULT 'grams' CHECK (unit IN ('grams', 'milliliters', 'cups', 'tablespoons', 'teaspoons', 'pieces', 'ounces', 'pounds', 'kilograms')),
    grams DOUBLE PRECISION NOT NULL CHECK (grams >= 0),
    preparation TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_recipe ON recipe_ingredients(recipe_id);
CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_fdc ON recipe_ingredients(fdc_id) WHERE fdc_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_order ON recipe_ingredients(recipe_id, sort_order);

-- ============================================================================
-- prompt_suggestions
-- ============================================================================
CREATE TABLE IF NOT EXISTS prompt_suggestions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    category_key TEXT NOT NULL,
    category_title TEXT NOT NULL,
    category_icon TEXT NOT NULL,
    pillar TEXT NOT NULL CHECK (pillar IN ('activity', 'nutrition', 'recovery')),
    prompts TEXT NOT NULL,
    display_order INTEGER NOT NULL DEFAULT 0,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(tenant_id, category_key),
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_prompt_suggestions_tenant ON prompt_suggestions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_prompt_suggestions_active ON prompt_suggestions(tenant_id, is_active);
CREATE INDEX IF NOT EXISTS idx_prompt_suggestions_order ON prompt_suggestions(tenant_id, display_order);

-- ============================================================================
-- welcome_prompts
-- ============================================================================
CREATE TABLE IF NOT EXISTS welcome_prompts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL UNIQUE REFERENCES tenants(id) ON DELETE CASCADE,
    prompt_text TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_welcome_prompts_tenant ON welcome_prompts(tenant_id);

-- ============================================================================
-- system_prompts
-- ============================================================================
CREATE TABLE IF NOT EXISTS system_prompts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL UNIQUE REFERENCES tenants(id) ON DELETE CASCADE,
    prompt_text TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_system_prompts_tenant ON system_prompts(tenant_id);

-- ============================================================================
-- stretching_exercises
-- ============================================================================
CREATE TABLE IF NOT EXISTS stretching_exercises (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT NOT NULL,
    difficulty TEXT NOT NULL DEFAULT 'beginner',
    primary_muscles TEXT NOT NULL,
    secondary_muscles TEXT,
    duration_seconds INTEGER NOT NULL DEFAULT 30,
    repetitions INTEGER,
    sets INTEGER DEFAULT 1,
    recommended_for_activities TEXT,
    contraindications TEXT,
    instructions TEXT NOT NULL,
    cues TEXT,
    image_url TEXT,
    video_url TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_stretching_category ON stretching_exercises(category);
CREATE INDEX IF NOT EXISTS idx_stretching_difficulty ON stretching_exercises(difficulty);
CREATE INDEX IF NOT EXISTS idx_stretching_name ON stretching_exercises(name);

-- ============================================================================
-- yoga_poses
-- ============================================================================
CREATE TABLE IF NOT EXISTS yoga_poses (
    id TEXT PRIMARY KEY,
    english_name TEXT NOT NULL,
    sanskrit_name TEXT,
    description TEXT NOT NULL,
    benefits TEXT NOT NULL,
    category TEXT NOT NULL,
    difficulty TEXT NOT NULL DEFAULT 'beginner',
    pose_type TEXT NOT NULL,
    primary_muscles TEXT NOT NULL,
    secondary_muscles TEXT,
    chakras TEXT,
    hold_duration_seconds INTEGER NOT NULL DEFAULT 30,
    breath_guidance TEXT,
    recommended_for_activities TEXT,
    recommended_for_recovery TEXT,
    contraindications TEXT,
    instructions TEXT NOT NULL,
    modifications TEXT,
    progressions TEXT,
    cues TEXT,
    warmup_poses TEXT,
    followup_poses TEXT,
    image_url TEXT,
    video_url TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_yoga_category ON yoga_poses(category);
CREATE INDEX IF NOT EXISTS idx_yoga_difficulty ON yoga_poses(difficulty);
CREATE INDEX IF NOT EXISTS idx_yoga_pose_type ON yoga_poses(pose_type);
CREATE INDEX IF NOT EXISTS idx_yoga_english_name ON yoga_poses(english_name);
CREATE INDEX IF NOT EXISTS idx_yoga_sanskrit_name ON yoga_poses(sanskrit_name);

-- ============================================================================
-- activity_muscle_mapping
-- ============================================================================
CREATE TABLE IF NOT EXISTS activity_muscle_mapping (
    id TEXT PRIMARY KEY,
    activity_type TEXT NOT NULL UNIQUE,
    primary_muscles TEXT NOT NULL,
    secondary_muscles TEXT,
    recommended_stretch_categories TEXT,
    recommended_yoga_categories TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_activity_muscle_type ON activity_muscle_mapping(activity_type);

-- ============================================================================
-- synthetic_activities
-- ============================================================================
CREATE TABLE IF NOT EXISTS synthetic_activities (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    sport_type TEXT NOT NULL,
    start_date TIMESTAMPTZ NOT NULL,
    duration_seconds INTEGER NOT NULL,
    distance_meters DOUBLE PRECISION,
    elevation_gain DOUBLE PRECISION,
    average_heart_rate INTEGER,
    max_heart_rate INTEGER,
    average_speed DOUBLE PRECISION,
    max_speed DOUBLE PRECISION,
    calories INTEGER,
    city TEXT,
    region TEXT,
    country TEXT,
    start_latitude DOUBLE PRECISION,
    start_longitude DOUBLE PRECISION,
    temperature DOUBLE PRECISION,
    humidity DOUBLE PRECISION,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_synthetic_activities_user_id ON synthetic_activities(user_id);
CREATE INDEX IF NOT EXISTS idx_synthetic_activities_tenant_id ON synthetic_activities(tenant_id);
CREATE INDEX IF NOT EXISTS idx_synthetic_activities_start_date ON synthetic_activities(start_date DESC);
CREATE INDEX IF NOT EXISTS idx_synthetic_activities_sport_type ON synthetic_activities(sport_type);

-- ============================================================================
-- friend_connections
-- ============================================================================
-- Bidirectional friend relationships with status tracking.
CREATE TABLE IF NOT EXISTS friend_connections (
    id UUID PRIMARY KEY,
    initiator_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    receiver_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'declined', 'blocked')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    UNIQUE(initiator_id, receiver_id),
    CHECK (initiator_id != receiver_id)
);
CREATE INDEX IF NOT EXISTS idx_friend_connections_initiator ON friend_connections(initiator_id, status);
CREATE INDEX IF NOT EXISTS idx_friend_connections_receiver ON friend_connections(receiver_id, status);

-- ============================================================================
-- user_social_settings
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_social_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    discoverable BOOLEAN NOT NULL DEFAULT TRUE,
    default_visibility TEXT NOT NULL DEFAULT 'friends_only'
        CHECK (default_visibility IN ('friends_only', 'public')),
    share_activity_types TEXT NOT NULL DEFAULT '["run", "ride", "swim"]',
    notify_friend_requests BOOLEAN NOT NULL DEFAULT TRUE,
    notify_insight_reactions BOOLEAN NOT NULL DEFAULT TRUE,
    notify_adapted_insights BOOLEAN NOT NULL DEFAULT TRUE,
    insight_sharing_policy TEXT NOT NULL DEFAULT 'friends_only',
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

-- ============================================================================
-- shared_insights
-- ============================================================================
-- Coach-mediated insights that users share with friends.
-- Privacy-first: no GPS, routes, exact paces. Only relative/contextual info.
CREATE TABLE IF NOT EXISTS shared_insights (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    visibility TEXT NOT NULL DEFAULT 'friends_only'
        CHECK (visibility IN ('friends_only', 'public')),
    insight_type TEXT NOT NULL
        CHECK (insight_type IN ('achievement', 'milestone', 'training_tip', 'recovery', 'motivation', 'coaching_insight')),
    sport_type TEXT,
    content TEXT NOT NULL,
    title TEXT,
    training_phase TEXT,
    reaction_count INTEGER NOT NULL DEFAULT 0,
    adapt_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_shared_insights_user ON shared_insights(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_shared_insights_friends_feed ON shared_insights(visibility, created_at DESC) WHERE visibility = 'friends_only';
CREATE INDEX IF NOT EXISTS idx_shared_insights_public_feed ON shared_insights(created_at DESC) WHERE visibility = 'public';
CREATE INDEX IF NOT EXISTS idx_shared_insights_type ON shared_insights(insight_type, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_shared_insights_sport ON shared_insights(sport_type, created_at DESC) WHERE sport_type IS NOT NULL;

-- ============================================================================
-- insight_reactions
-- ============================================================================
CREATE TABLE IF NOT EXISTS insight_reactions (
    id TEXT PRIMARY KEY,
    insight_id TEXT NOT NULL REFERENCES shared_insights(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reaction_type TEXT NOT NULL
        CHECK (reaction_type IN ('like', 'celebrate', 'inspire', 'support')),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(insight_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_insight_reactions_insight ON insight_reactions(insight_id, reaction_type);
CREATE INDEX IF NOT EXISTS idx_insight_reactions_user ON insight_reactions(user_id, created_at DESC);

-- ============================================================================
-- adapted_insights
-- ============================================================================
-- Personalized versions of friend insights adapted to the requesting user's context.
CREATE TABLE IF NOT EXISTS adapted_insights (
    id TEXT PRIMARY KEY,
    source_insight_id TEXT NOT NULL REFERENCES shared_insights(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    adapted_content TEXT NOT NULL,
    adaptation_context TEXT,
    was_helpful BOOLEAN,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(source_insight_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_adapted_insights_user ON adapted_insights(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_adapted_insights_source ON adapted_insights(source_insight_id);

-- ============================================================================
-- PostgreSQL trigger functions for social table denormalized counts
-- ============================================================================
CREATE OR REPLACE FUNCTION update_shared_insight_reaction_count()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE shared_insights
        SET reaction_count = reaction_count + 1, updated_at = NOW()
        WHERE id = NEW.insight_id;
    ELSIF TG_OP = 'DELETE' THEN
        UPDATE shared_insights
        SET reaction_count = reaction_count - 1, updated_at = NOW()
        WHERE id = OLD.insight_id;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION update_shared_insight_adapt_count()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE shared_insights
        SET adapt_count = adapt_count + 1, updated_at = NOW()
        WHERE id = NEW.source_insight_id;
    ELSIF TG_OP = 'DELETE' THEN
        UPDATE shared_insights
        SET adapt_count = adapt_count - 1, updated_at = NOW()
        WHERE id = OLD.source_insight_id;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_insight_reactions_change ON insight_reactions;
CREATE TRIGGER trg_insight_reactions_change
    AFTER INSERT OR DELETE ON insight_reactions
    FOR EACH ROW EXECUTE FUNCTION update_shared_insight_reaction_count();

DROP TRIGGER IF EXISTS trg_adapted_insights_change ON adapted_insights;
CREATE TRIGGER trg_adapted_insights_change
    AFTER INSERT OR DELETE ON adapted_insights
    FOR EACH ROW EXECUTE FUNCTION update_shared_insight_adapt_count();
