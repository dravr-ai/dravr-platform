-- ABOUTME: Drop the unused users.billing_test column (SQLite)
-- ABOUTME: Added in 20260424000002 but never read in code; quota bypass uses QUOTA_BYPASS_USER_IDS env instead

ALTER TABLE users DROP COLUMN billing_test;
