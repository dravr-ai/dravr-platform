-- ABOUTME: One-time normalization: align users.is_admin with users.role
-- ABOUTME: Fixes skewed rows where is_admin=1 but role='user' (or vice versa)
--
-- See migrations_pg/20260518000001_normalize_user_is_admin.sql for the
-- rationale. SQLite stores BOOLEAN as INTEGER (0/1).
UPDATE users
   SET is_admin = CASE WHEN role IN ('admin', 'super_admin') THEN 1 ELSE 0 END
 WHERE is_admin <> CASE WHEN role IN ('admin', 'super_admin') THEN 1 ELSE 0 END;
