-- ABOUTME: Records who granted manages_roster as an operator, and when, so the TrainingPeaks reconciler never takes it back
-- ABOUTME: Written by the admin roster route and pierre-cli user set; read by the reconciler's revoke and the admin views
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- When an operator granted manages_roster; NULL when no operator grant holds.
-- The TrainingPeaks coach connection that earns the grant on its own revokes
-- only a grant this is NULL for.
ALTER TABLE users ADD COLUMN IF NOT EXISTS manages_roster_granted_at TIMESTAMPTZ;
-- The operator who granted it; NULL for a service token, which names no one,
-- and once that operator's account is deleted.
ALTER TABLE users ADD COLUMN IF NOT EXISTS manages_roster_granted_by UUID REFERENCES users(id) ON DELETE SET NULL;
