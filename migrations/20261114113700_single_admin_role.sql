-- ABOUTME: Folds the super_admin user role into admin: there is one admin role and it holds every power (carnet#1567)
-- ABOUTME: Data only, behind a precondition: refuses to run while an account holds is_admin without the admin role (carnet#1574)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An admin was meant to be all-powerful from the first day; the super_admin
-- tier above it was a split nobody asked for. The build that lands with this
-- knows two roles, 'user' and 'admin', and reads any other stored value as a
-- plain user, so every super_admin row is rewritten before the server serves.
-- `is_admin` is set with it: the two columns are one fact (20260518000001).
--
-- The CHECK on users.role still lists 'super_admin', so that a revision from
-- before the fold, still serving during a deploy or rolled back to, is not
-- refused when it writes the value. It is not stored as written either: from
-- 20261114113701 on, a written 'super_admin' is stored as 'admin'.
--
-- ONE-WAY. A rollback does not undo this statement, and under the older
-- build 'admin' is its lower tier: nobody holds the powers it reserves to
-- 'super_admin' (impersonation, admin management, tenant plans, billing
-- export, cross-tenant reads, browser approval of a `pierre-cli` login, the
-- admin mail) and that build cannot recreate the tier while the trigger of
-- 20261114113701 exists. The procedure that gives an older build its
-- 'super_admin' accounts back is in that migration's header.
--
-- `admin_tokens.is_super_admin` is untouched. It is a scope stored on a
-- service token, not a user role.

-- Precondition, checked before anything is written. The build this replaces
-- treated `is_admin = true, role = 'user'` as an admin; this one decides
-- from `role` alone, and 20261114113701 clears the flag of such a row. That
-- is how 20260518000001 demoted both founders. If the last admin in good
-- standing were such a row, nobody would hold the role and the public
-- first-admin setup (`POST /admin/setup`) would accept a new one. So the
-- migration refuses to run instead of deciding: each migration is its own
-- transaction, and failing here leaves the database exactly as the serving
-- revision reads it.
-- Settle each row by hand, then deploy again:
--   UPDATE users SET role = 'admin' WHERE id = ...;   -- it is an admin
--   UPDATE users SET is_admin = 0 WHERE id = ...;     -- it is not
--
-- SQLite has no RAISE outside a trigger body, so the refusal is a trigger on
-- a temporary table: the insert below fires it, and its ABORT fails the
-- migration with the message. Both go when the table is dropped; IF NOT
-- EXISTS is for a connection that was refused once outside a transaction and
-- still holds them.
CREATE TEMP TABLE IF NOT EXISTS single_admin_role_precondition (checked INTEGER);

CREATE TEMP TRIGGER IF NOT EXISTS single_admin_role_refuses_flag_only_admins
BEFORE INSERT ON single_admin_role_precondition
WHEN EXISTS (SELECT 1 FROM users WHERE is_admin AND role NOT IN ('admin', 'super_admin'))
BEGIN
    SELECT RAISE(ABORT, 'single admin role (carnet#1574): an account holds is_admin without the admin role; set role = ''admin'' or clear is_admin on each, then run the migration again');
END;

INSERT INTO single_admin_role_precondition (checked) VALUES (1);

DROP TABLE IF EXISTS single_admin_role_precondition;

UPDATE users SET role = 'admin', is_admin = 1 WHERE role = 'super_admin';
