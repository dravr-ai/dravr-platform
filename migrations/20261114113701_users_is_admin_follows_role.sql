-- ABOUTME: users.role is the one source of truth for who is an admin; users.is_admin is derived from it on every write (carnet#1574)
-- ABOUTME: Triggers that rewrite a row instead of refusing it: a stored 'super_admin' becomes 'admin', and is_admin becomes (role = 'admin')

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- `users.is_admin` and `users.role` said the same thing in two columns and
-- nothing kept them in step. 20260518000001 reconciled them once by trusting
-- `role`, and so demoted both founders: their rows were `is_admin = 1,
-- role = 'user'`, written by a first-admin setup that set the flag alone.
-- From here the role decides, in the database: whatever a writer sends,
-- the stored `is_admin` is (role = 'admin').
--
-- The triggers rewrite; they never refuse. 20261114113700 leaves
-- 'super_admin' in the role CHECK so that a revision from before the fold,
-- still serving during a deploy or rolled back to, is not refused when it
-- writes it. Refusing that write would fail that revision's boot seeder, and
-- storing it as written would read as a plain user in this build: the
-- lockout again. So a written 'super_admin' is stored as 'admin'.
--
-- ONE-WAY, and what a rollback past this migration costs: the older build
-- reads 'admin' as its lower tier and cannot store 'super_admin' while these
-- triggers exist, so no account holds its 'super_admin' role until someone
-- restores the tier by hand. The deployed database is PostgreSQL; the full account of what is
-- lost and the procedure are in the header of the migrations_pg file of the
-- same name. Here the two triggers to drop are
-- trg_users_is_admin_follows_role_insert and
-- trg_users_is_admin_follows_role_update, and the rows to delete from
-- _sqlx_migrations are the same two versions, so that this build's
-- migrator runs both again before it serves.
--
-- SQLite cannot assign to NEW, so each trigger runs after the write and
-- updates the row it was fired for. The WHEN clause is the mismatch itself,
-- which the trigger's own UPDATE removes: with `recursive_triggers` off (the
-- default, and what the pool runs) that UPDATE does not fire the trigger
-- again, and with it on it fires once more, finds no mismatch and stops.
-- A CHECK would need a table rebuild on SQLite; the table is not rebuilt.

CREATE TRIGGER IF NOT EXISTS trg_users_is_admin_follows_role_insert
AFTER INSERT ON users
FOR EACH ROW
WHEN NEW.role = 'super_admin' OR NEW.is_admin IS NOT (NEW.role = 'admin')
BEGIN
    UPDATE users
    SET role = CASE WHEN NEW.role = 'super_admin' THEN 'admin' ELSE NEW.role END,
        is_admin = (NEW.role IN ('admin', 'super_admin'))
    WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_users_is_admin_follows_role_update
AFTER UPDATE ON users
FOR EACH ROW
WHEN NEW.role = 'super_admin' OR NEW.is_admin IS NOT (NEW.role = 'admin')
BEGIN
    UPDATE users
    SET role = CASE WHEN NEW.role = 'super_admin' THEN 'admin' ELSE NEW.role END,
        is_admin = (NEW.role IN ('admin', 'super_admin'))
    WHERE id = NEW.id;
END;

-- The rows already stored are brought to the same rule, after the triggers
-- exist so that no write can land between the two. The role decides, in both
-- directions, and the second statement would make a row with `is_admin = 1,
-- role = 'user'` a plain user: the rule 20260518000001 applied to the
-- founders while their admin standing was held by the flag alone. It demotes
-- nobody here, and not on anyone's word: 20261114113700 refuses to run while
-- such a row exists, so the only rows left out of step are admins by role
-- whose flag is off.
UPDATE users SET role = 'admin' WHERE role = 'super_admin';
UPDATE users SET is_admin = (role = 'admin') WHERE is_admin IS NOT (role = 'admin');
