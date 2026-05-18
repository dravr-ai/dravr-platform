-- ABOUTME: One-time normalization: align users.is_admin with users.role
-- ABOUTME: Fixes skewed rows where is_admin=true but role='user' (or vice versa)
--
-- Context: routes/admin/setup.rs:95 creates the first admin user by setting
-- `admin_user.is_admin = true` on a User built from User::new(), which
-- defaults role to UserRole::User. The DB therefore stores is_admin=true /
-- role='user'. The read-side reconciliation in
-- crates/pierre-database/src/database/users.rs:266-278 papers over this on
-- some code paths but not all — the web SPA's session response ships the
-- raw values, producing a USER role badge with an admin-hidden Settings
-- tab list (Data Providers / Messaging / About). UserSettings.tsx gates on
-- is_admin while Dashboard.tsx gates on role; when the two disagree the UI
-- is inconsistent.
--
-- This backfill makes the two columns agree by treating `role` as the
-- canonical source of truth (it carries strictly more information:
-- user/admin/super_admin vs the boolean is_admin).
--
-- Scope: every row where is_admin and role disagree. Touched 2 rows in
-- prod at the time of writing (jf@dravr.ai, phil@dravr.ai).
UPDATE users
   SET is_admin = (role IN ('admin', 'super_admin'))
 WHERE is_admin <> (role IN ('admin', 'super_admin'));
