-- ABOUTME: users.role is the one source of truth for who is an admin; users.is_admin is derived from it on every write (carnet#1574)
-- ABOUTME: A trigger that rewrites a row instead of refusing it: a stored 'super_admin' becomes 'admin', and is_admin becomes (role = 'admin')

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- `users.is_admin` and `users.role` said the same thing in two columns and
-- nothing kept them in step. 20260518000001 reconciled them once by trusting
-- `role`, and so demoted both founders: their rows were `is_admin = true,
-- role = 'user'`, written by a first-admin setup that set the flag alone.
-- From here the role decides, in the database: whatever a writer sends,
-- the stored `is_admin` is (role = 'admin').
--
-- The trigger rewrites; it never refuses. 20261114113700 leaves
-- 'super_admin' in the role CHECK so that a revision from before the fold,
-- still serving during a deploy or rolled back to, is not refused when it
-- writes it. Refusing that write would fail that revision's boot seeder, and
-- storing it as written would read as a plain user in this build: the
-- lockout again. So a written 'super_admin' is stored as 'admin'. A BEFORE
-- row trigger sets NEW, so the row is stored once, already in step.
--
-- ONE-WAY, and what a rollback past this migration costs. The older build
-- reads 'admin' as its lower tier and cannot store 'super_admin' while this
-- trigger exists: its seeder and its `pierre-cli user create --super-admin`
-- both end up with an 'admin' row. Until the steps below are run, no account
-- of that build holds 'super_admin': no impersonation, admin management,
-- tenant plan, billing export or cross-tenant read, no browser approval of a
-- new `pierre-cli auth login`, and nobody receives the sign-up and
-- coach-access mail. The same holds, for the minutes they keep serving, on the instances
-- of the old revision during the deploy. Nothing fails; the powers are
-- missing.
--
-- To give a rolled-back build its 'super_admin' accounts back, as the
-- database owner:
--   1. DROP TRIGGER trg_users_is_admin_follows_role ON users;
--      DROP FUNCTION users_is_admin_follows_role();
--   2. UPDATE users SET role = 'super_admin', is_admin = TRUE
--        WHERE email IN (...);          -- the accounts that held the tier
--   3. DELETE FROM _sqlx_migrations
--        WHERE version IN (20261114113700, 20261114113701);
-- Step 3 is what makes deploying this build again safe. The migrator applies
-- whatever version has no row, so both migrations run again on its boot:
-- the fold rewrites the restored 'super_admin' rows and this trigger comes
-- back. Without it the migrator sees both as applied, the trigger stays
-- dropped, and this build reads every 'super_admin' row as a plain user:
-- with no admin left, the public first-admin setup would accept a new one.
CREATE OR REPLACE FUNCTION users_is_admin_follows_role()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.role = 'super_admin' THEN
        NEW.role := 'admin';
    END IF;
    NEW.is_admin := (NEW.role = 'admin');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_users_is_admin_follows_role ON users;
CREATE TRIGGER trg_users_is_admin_follows_role
    BEFORE INSERT OR UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION users_is_admin_follows_role();

-- The rows already stored are brought to the same rule, after the trigger
-- exists: creating it locks out writers until this migration commits, so no
-- write can land between the two. The role decides, in both directions, and
-- the second statement would make a row with `is_admin = true, role = 'user'`
-- a plain user: the rule 20260518000001 applied to the founders while their
-- admin standing was held by the flag alone. It demotes nobody here, and not
-- on anyone's word: 20261114113700 refuses to run while such a row exists,
-- so the only rows left out of step are admins by role whose flag is off.
UPDATE users SET role = 'admin' WHERE role = 'super_admin';
UPDATE users SET is_admin = (role = 'admin') WHERE is_admin <> (role = 'admin');
