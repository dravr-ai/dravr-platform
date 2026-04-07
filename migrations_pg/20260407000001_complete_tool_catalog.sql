-- ABOUTME: Adds missing tool categories and seeds 32 tools absent from the catalog
-- ABOUTME: Closes the gap between the 47-entry catalog and the 79-tool backend registry

-- Step 1: Replace the category CHECK constraint to include new categories
ALTER TABLE tool_catalog DROP CONSTRAINT IF EXISTS tool_catalog_category_check;
ALTER TABLE tool_catalog ADD CONSTRAINT tool_catalog_category_check
    CHECK (category IN (
        'fitness', 'analysis', 'goals', 'nutrition',
        'recipes', 'sleep', 'configuration', 'connections',
        'coaches', 'admin', 'mobility'
    ));

-- Step 2: Seed the 32 missing tools

-- Analytics: analyze_weather_impact (was in registry but missing from catalog)
INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-048', 'analyze_weather_impact', 'Weather Impact', 'Analyze how weather conditions affected activity performance, including temperature, humidity, wind, and precipitation impact', 'analysis', TRUE, NULL, 'professional')
ON CONFLICT (id) DO NOTHING;

-- Coach tools (13 tools)
INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-049', 'list_coaches', 'List Coaches', 'List available AI coaches for personalized training guidance', 'coaches', TRUE, NULL, 'starter'),
('tc-050', 'create_coach', 'Create Coach', 'Create a custom AI coach with personalized training guidance', 'coaches', TRUE, NULL, 'starter'),
('tc-051', 'get_coach', 'Get Coach', 'Get detailed information about a specific coach', 'coaches', TRUE, NULL, 'starter'),
('tc-052', 'update_coach', 'Update Coach', 'Update an existing coach settings', 'coaches', TRUE, NULL, 'starter'),
('tc-053', 'delete_coach', 'Delete Coach', 'Delete a coach', 'coaches', TRUE, NULL, 'starter'),
('tc-054', 'toggle_coach_favorite', 'Toggle Coach Favorite', 'Toggle the favorite status of a coach', 'coaches', TRUE, NULL, 'starter'),
('tc-055', 'search_coaches', 'Search Coaches', 'Search for coaches by query', 'coaches', TRUE, NULL, 'starter'),
('tc-056', 'activate_coach', 'Activate Coach', 'Activate a coach for personalized training guidance', 'coaches', TRUE, NULL, 'starter'),
('tc-057', 'deactivate_coach', 'Deactivate Coach', 'Deactivate the current coach and return to default AI guidance', 'coaches', TRUE, NULL, 'starter'),
('tc-058', 'get_active_coach', 'Get Active Coach', 'Get the currently active coach', 'coaches', TRUE, NULL, 'starter'),
('tc-059', 'hide_coach', 'Hide Coach', 'Hide a coach from listings', 'coaches', TRUE, NULL, 'starter'),
('tc-060', 'show_coach', 'Show Coach', 'Show a previously hidden coach', 'coaches', TRUE, NULL, 'starter'),
('tc-061', 'list_hidden_coaches', 'List Hidden Coaches', 'List all hidden coaches', 'coaches', TRUE, NULL, 'starter')
ON CONFLICT (id) DO NOTHING;

-- Admin tools (8 tools)
INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-062', 'admin_list_system_coaches', 'List System Coaches', 'List all system coaches in the tenant (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-063', 'admin_create_system_coach', 'Create System Coach', 'Create a new system coach visible to all tenant users (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-064', 'admin_get_system_coach', 'Get System Coach', 'Get detailed information about a system coach (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-065', 'admin_update_system_coach', 'Update System Coach', 'Update an existing system coach (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-066', 'admin_delete_system_coach', 'Delete System Coach', 'Delete a system coach and remove all assignments (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-067', 'admin_assign_coach', 'Assign Coach', 'Assign a system coach to a specific user (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-068', 'admin_unassign_coach', 'Unassign Coach', 'Remove a coach assignment from a user (admin only)', 'admin', TRUE, NULL, 'starter'),
('tc-069', 'admin_list_coach_assignments', 'List Coach Assignments', 'List all assignments for a system coach (admin only)', 'admin', TRUE, NULL, 'starter')
ON CONFLICT (id) DO NOTHING;

-- Mobility tools (6 tools)
INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-070', 'list_stretching_exercises', 'List Stretching Exercises', 'List stretching exercises with optional filtering by category, difficulty, muscle group, or activity type', 'mobility', TRUE, NULL, 'starter'),
('tc-071', 'get_stretching_exercise', 'Get Stretching Exercise', 'Get detailed information about a specific stretching exercise', 'mobility', TRUE, NULL, 'starter'),
('tc-072', 'suggest_stretches_for_activity', 'Suggest Stretches', 'Get personalized stretching recommendations based on your recent activity type', 'mobility', TRUE, NULL, 'starter'),
('tc-073', 'list_yoga_poses', 'List Yoga Poses', 'List yoga poses with optional filtering by category, difficulty, pose type, or recovery context', 'mobility', TRUE, NULL, 'starter'),
('tc-074', 'get_yoga_pose', 'Get Yoga Pose', 'Get detailed information about a specific yoga pose', 'mobility', TRUE, NULL, 'starter'),
('tc-075', 'suggest_yoga_sequence', 'Suggest Yoga Sequence', 'Create a personalized yoga sequence for recovery based on your recent activities or goals', 'mobility', TRUE, NULL, 'starter')
ON CONFLICT (id) DO NOTHING;

-- Health data tools (4 tools)
INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-076', 'get_sleep_sessions', 'Sleep Sessions', 'Get stored sleep sessions from the health data database', 'sleep', TRUE, NULL, 'starter'),
('tc-077', 'get_recovery_metrics', 'Recovery Metrics', 'Get stored recovery and readiness metrics from the health data database', 'sleep', TRUE, NULL, 'starter'),
('tc-078', 'get_health_snapshots', 'Health Snapshots', 'Get stored health snapshots including body composition and vitals', 'fitness', TRUE, NULL, 'starter'),
('tc-079', 'list_data_sources', 'Data Sources', 'List connected data sources including devices and providers', 'connections', TRUE, NULL, 'starter')
ON CONFLICT (id) DO NOTHING;
