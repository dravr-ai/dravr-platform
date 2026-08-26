-- ABOUTME: Deletes the stored rows of the Social notification category retired by the Chat-First Cutover.
-- ABOUTME: Preferences and persisted notifications both go — dravr-commere 0.2.0 no longer parses 'social'.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- NotificationCategory::Social left dravr-commere in 0.2.0 together with the
-- four social triggers (friend requests, kudos, shared insights), because the
-- Insights and Friends surfaces they deep-linked into were deleted from every
-- client. A stored 'social' string now fails from_str_opt on read, which would
-- error the whole preference list and notification feed of any user carrying
-- one, so the rows are deleted rather than re-pointed: there is no surface
-- left for a social notification to open.
DELETE FROM notification_preferences WHERE category = 'social';
DELETE FROM notifications WHERE category = 'social';
