-- ABOUTME: Deletes the stored rows of the AI Insights and Reminders notification categories retired on carnet#1436.
-- ABOUTME: Preferences and persisted notifications both go, as 20260826000006 did for Social, and every scheduled notification goes with them.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The AI Insights and Reminders tabs and settings rows are gone from every
-- client. A row under either has no switch left to change it and no tab to read
-- it from, so it is deleted rather than re-pointed. NotificationCategory::Ai and
-- ::Reminders stay in dravr-commere until this has been deployed, so a binary
-- still on the old pin keeps parsing whatever it reads during the rollout.
DELETE FROM notification_preferences WHERE category IN ('ai', 'reminders');
DELETE FROM notifications WHERE category IN ('ai', 'reminders');

-- Nothing can create or manage a schedule any more, and dravr-commere's scheduler
-- would write a notification from any row left here when it comes due, so every
-- row goes, whatever its type. The table itself stays.
DELETE FROM scheduled_notifications;
