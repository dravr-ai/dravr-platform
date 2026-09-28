-- ABOUTME: Emails are case-insensitive: every stored email lowercased and trimmed, case variants refused by index (SQLite)
-- ABOUTME: Refuses to run, naming the addresses, when lowercasing would merge two accounts or two pre-approvals

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An email names one person whatever its case (decided 2026-09-27). The
-- server stored an address as typed and compared it exactly, so `Jane@x.com`
-- registered one account and `jane@x.com` another, a Google sign-in whose
-- provider reported another casing started a second account, and a sign-in
-- in another casing than the registration found nothing. Every write now
-- stores the trimmed, lowercase form and every lookup compares lower-cased;
-- this brings the stored rows to that form and makes the unique columns
-- refuse a case variant from here on.

-- 1. Refuse on a collision. Lowercasing two rows of a unique column onto one
-- address would merge two accounts (or two pre-approvals); that is a human
-- decision, so nothing is merged or dropped here and the migration fails,
-- naming every address involved, until an operator merges them by hand.
-- SQLite has no RAISE outside a trigger, so the refusal is a JSON path error
-- whose text is the report: json_extract rejects a path that does not start
-- with '$' with "bad JSON path: '<the path>'". The statement returns no row,
-- and so evaluates nothing, when there is no collision.
SELECT json_extract('{}',
    'users.email case collision: merge these accounts by hand, then migrate again: '
    || group_concat(variants, '; '))
FROM (
    SELECT group_concat(email, ' = ') AS variants
    FROM users
    GROUP BY lower(trim(email))
    HAVING count(*) > 1
)
HAVING count(*) > 0;

SELECT json_extract('{}',
    'pre_approved_emails.email case collision: remove all but one of each by hand, then migrate again: '
    || group_concat(variants, '; '))
FROM (
    SELECT group_concat(email, ' = ') AS variants
    FROM pre_approved_emails
    GROUP BY lower(trim(email))
    HAVING count(*) > 1
)
HAVING count(*) > 0;

-- 2. Every email column, to its stored form.
UPDATE users SET email = lower(trim(email))
WHERE email <> lower(trim(email));

UPDATE pre_approved_emails SET email = lower(trim(email))
WHERE email <> lower(trim(email));

UPDATE delegated_connections SET provider_athlete_email = lower(trim(provider_athlete_email))
WHERE provider_athlete_email <> lower(trim(provider_athlete_email));

UPDATE messaging_link_states SET email = lower(trim(email))
WHERE email <> lower(trim(email));

UPDATE admin_provisioned_keys SET user_email = lower(trim(user_email))
WHERE user_email <> lower(trim(user_email));

UPDATE admin_config_audit SET admin_email = lower(trim(admin_email))
WHERE admin_email <> lower(trim(admin_email));

UPDATE a2a_clients SET contact_email = lower(trim(contact_email))
WHERE contact_email <> lower(trim(contact_email));

-- 3. A case variant of a stored address can never be inserted again. Lookups
-- compare lower(email) = lower($1), which these indexes serve. The plain
-- users.email UNIQUE constraint stays: the demo seeder's ON CONFLICT(email)
-- names it. idx_users_email duplicated that constraint's own index and no
-- lookup reads the bare column any more.
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email_lower ON users (lower(email));
CREATE UNIQUE INDEX IF NOT EXISTS idx_pre_approved_emails_email_lower
    ON pre_approved_emails (lower(email));
DROP INDEX IF EXISTS idx_users_email;
