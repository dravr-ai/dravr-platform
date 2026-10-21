// ABOUTME: The team-membership migration: only real choices become memberships, unedited Store copies fold onto their catalogue agent
// ABOUTME: Replays each lane's migrations up to it, plants copies, usage rows and every reference, and reads where each row landed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1096. Until 20261020180000 one `agent_assignments` row meant "on the
//! team", "used once" and "favourite" at once, and a Store install of a
//! catalogue agent was a frozen personal copy. The migration marks the rows
//! that were a real choice (`is_member`) and moves every copy nobody edited
//! back onto its catalogue agent. It is proven on a real database of each
//! lane, because what it must NOT touch matters as much as what it moves:
//!
//! - a usage-only row, and the signup selection, never become a membership
//!   (carnet#1098);
//! - an owner's row, an admin assignment and a favourite do;
//! - an edited copy (a version row, or `updated_at` past `created_at`) stays,
//!   and so does a copy of an agent another tenant's author published — but a
//!   copy the pre-fix seeder rewrote (`source = 'contremaitre'`, no version
//!   row) moves although its `updated_at` moved;
//! - an unedited copy hands every reference to its origin — relations and the
//!   forks made from it included — and the athlete's rows for that origin
//!   collapse into one — the origin's own row when there is one, else the
//!   lowest-id copy row — even across tenants; it is a membership only when
//!   one of the folded rows was, so someone else's usage row on the copy
//!   stays usage;
//! - every listing's install count is re-read: members other than the author,
//!   plus the copies that stayed.
//!
//! `SQLite` applies this migration with foreign keys ON, as the production pool
//! does (`sqlx` connects with `foreign_keys = true`), so a reference the
//! migration forgot to move fails the delete here exactly as it would there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// The migration under test; everything before it builds the legacy shape.
const TEAM_MEMBERSHIP_MIGRATION: i64 = 20_261_020_180_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// The operator who owns the catalogue agents.
const OPERATOR: &str = "aaaaaaaa-0000-4000-8000-000000000001";
/// Installed a copy of `ORIGIN` and had also used `ORIGIN` itself.
const ATHLETE_A: &str = "aaaaaaaa-0000-4000-8000-000000000002";
/// Holds the copies that must stay: an edited one, a touched one, a foreign one.
const ATHLETE_B: &str = "aaaaaaaa-0000-4000-8000-000000000003";
/// Installed `ORIGIN2` twice, once in each of two tenants.
const ATHLETE_C: &str = "aaaaaaaa-0000-4000-8000-000000000004";
/// Only used `ORIGIN` (and signup selected it); an admin assigned `ORIGIN2`.
const USER_D: &str = "aaaaaaaa-0000-4000-8000-000000000005";
/// Favourited `ORIGIN`; used `ORIGIN2` once.
const USER_E: &str = "aaaaaaaa-0000-4000-8000-000000000006";
/// Wrote the athlete-published agent another tenant installed.
const AUTHOR: &str = "aaaaaaaa-0000-4000-8000-000000000007";

const ORIGIN: &str = "a0000000-0000-4000-8000-000000000001";
const ORIGIN2: &str = "a0000000-0000-4000-8000-000000000002";
const USER_PUB: &str = "a0000000-0000-4000-8000-000000000003";
const ORIGIN3: &str = "a0000000-0000-4000-8000-000000000004";
const COPY_A: &str = "c0000000-0000-4000-8000-000000000001";
const COPY_EDITED: &str = "c0000000-0000-4000-8000-000000000002";
const COPY_TOUCHED: &str = "c0000000-0000-4000-8000-000000000003";
const COPY_FOREIGN: &str = "c0000000-0000-4000-8000-000000000004";
const COPY_C1: &str = "c0000000-0000-4000-8000-000000000005";
const COPY_C2: &str = "c0000000-0000-4000-8000-000000000006";
const COPY_SEEDED: &str = "c0000000-0000-4000-8000-000000000007";
const COPY_OF_COPY: &str = "c0000000-0000-4000-8000-000000000008";
const COPY_LISTED: &str = "c0000000-0000-4000-8000-000000000009";

const A_HOME: &str = "11111111-0000-4000-8000-000000000002";
const B_HOME: &str = "11111111-0000-4000-8000-000000000003";
const C_HOME: &str = "11111111-0000-4000-8000-000000000004";
const C_SECOND: &str = "11111111-0000-4000-8000-000000000005";
const D_HOME: &str = "11111111-0000-4000-8000-000000000007";

/// What the two backends share once the users exist. Literals only — `TRUE`
/// and `FALSE` are spelled the way both engines accept — so the same text
/// plants either lane.
const FIXTURE: &str = r"
INSERT INTO tenants (id, name, slug, created_at, updated_at) VALUES
    ('11111111-0000-4000-8000-000000000001', 'Ops', 'ops', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000002', 'A', 'a-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000003', 'B', 'b-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000004', 'C', 'c-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000005', 'C2', 'c-second', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000006', 'Author', 'author-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000007', 'D', 'd-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, slug, is_system, forked_from, created_at, updated_at) VALUES
    ('a0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001',
     'Tempo Coach', 'You pace.', 'tempo-coach', TRUE, NULL, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('a0000000-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001',
     'Hill Coach', 'You climb.', 'hill-coach', TRUE, NULL, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('a0000000-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000007', '11111111-0000-4000-8000-000000000006',
     'River Coach', 'You paddle.', 'river-coach', FALSE, NULL, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, slug, is_system, forked_from, created_at, updated_at) VALUES
    -- Unedited copy of a catalogue agent: moves.
    ('c0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000002',
     'Tempo Coach', 'You pace.', 'tempo-coach', FALSE, 'a0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    -- Edited (it has a version row) though its timestamps match: stays.
    ('c0000000-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003',
     'Tempo Coach (mine)', 'You pace my way.', 'tempo-coach', FALSE, 'a0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    -- Touched after install: stays.
    ('c0000000-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003',
     'Tempo Coach', 'You pace.', 'tempo-coach', FALSE, 'a0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00', '2026-03-01T00:00:00+00:00'),
    -- Unedited copy of an athlete-published agent: the reviewed snapshot, stays.
    ('c0000000-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003',
     'River Coach', 'You paddle.', 'river-coach', FALSE, 'a0000000-0000-4000-8000-000000000003', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    -- One athlete, two unedited copies of one catalogue agent in two tenants: both move.
    ('c0000000-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000004', '11111111-0000-4000-8000-000000000004',
     'Hill Coach', 'You climb.', 'hill-coach', FALSE, 'a0000000-0000-4000-8000-000000000002', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    ('c0000000-0000-4000-8000-000000000006', 'aaaaaaaa-0000-4000-8000-000000000004', '11111111-0000-4000-8000-000000000005',
     'Hill Coach', 'You climb.', 'hill-coach', FALSE, 'a0000000-0000-4000-8000-000000000002', '2026-02-02T00:00:00+00:00', '2026-02-02T00:00:00+00:00');

-- A copy the pre-fix seeder rewrote by slug: updated_at moved and the source
-- says contremaitre, but no athlete edited it (no version row): moves.
INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, slug, is_system, forked_from, source, created_at, updated_at) VALUES
    ('c0000000-0000-4000-8000-000000000007', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003',
     'Hill Coach', 'You climb.', 'hill-coach', FALSE, 'a0000000-0000-4000-8000-000000000002', 'contremaitre',
     '2026-02-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00');

-- An edited fork of a moving copy: stays, and is forked from the origin after.
INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, slug, is_system, forked_from, created_at, updated_at) VALUES
    ('c0000000-0000-4000-8000-000000000008', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003',
     'Tempo Coach (fork)', 'You pace.', 'tempo-coach', FALSE, 'c0000000-0000-4000-8000-000000000001',
     '2026-02-05T00:00:00+00:00', '2026-02-06T00:00:00+00:00');

-- A catalogue agent with no listing of its own, and an unedited copy of it
-- the seeder wrote a listing on: the listing moves to the origin.
INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, slug, is_system, forked_from, created_at, updated_at) VALUES
    ('a0000000-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001',
     'Swim Coach', 'You swim.', 'swim-coach', TRUE, NULL, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('c0000000-0000-4000-8000-000000000009', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003',
     'Swim Coach', 'You swim.', 'swim-coach', FALSE, 'a0000000-0000-4000-8000-000000000004', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO agent_relations (id, agent_id, related_agent_id, relation_type, created_at) VALUES
    -- Another agent's relation that resolved the copy: follows it to the origin.
    ('rel-1', 'a0000000-0000-4000-8000-000000000003', 'c0000000-0000-4000-8000-000000000001', 'related', '2026-02-01T00:00:00+00:00'),
    -- A relation the seeder wrote on the copy: the origin takes it.
    ('rel-2', 'c0000000-0000-4000-8000-000000000007', 'a0000000-0000-4000-8000-000000000003', 'alternative', '2026-02-01T00:00:00+00:00'),
    -- The copy related to its own origin: would relate the origin to itself, goes.
    ('rel-3', 'c0000000-0000-4000-8000-000000000001', 'a0000000-0000-4000-8000-000000000001', 'related', '2026-02-01T00:00:00+00:00');

INSERT INTO agent_versions (id, agent_id, version, content_hash, content_snapshot, created_at) VALUES
    ('ver-edited', 'c0000000-0000-4000-8000-000000000002', 1, 'h1', '{}', '2026-02-01T00:00:00+00:00');

INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at, selected_agent_id) VALUES
    ('33333333-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', NULL),
    ('33333333-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', 'c0000000-0000-4000-8000-000000000001'),
    ('33333333-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', 'c0000000-0000-4000-8000-000000000002'),
    ('33333333-0000-4000-8000-000000000004', '11111111-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000004', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', 'c0000000-0000-4000-8000-000000000005'),
    ('33333333-0000-4000-8000-000000000005', '11111111-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000004', 'member', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', 'c0000000-0000-4000-8000-000000000006'),
    ('33333333-0000-4000-8000-000000000006', '11111111-0000-4000-8000-000000000006', 'aaaaaaaa-0000-4000-8000-000000000007', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', NULL),
    -- Signup selected the starter agent for D: a selection, not a membership.
    ('33333333-0000-4000-8000-000000000007', '11111111-0000-4000-8000-000000000007', 'aaaaaaaa-0000-4000-8000-000000000005', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', 'a0000000-0000-4000-8000-000000000001');

INSERT INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at) VALUES
    -- A used the catalogue agent itself, then installed a copy of it: one membership.
    ('as-01', 'a0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002',
     '2026-01-02T00:00:00+00:00', FALSE, 2, '2026-03-01T00:00:00+00:00'),
    ('as-02', 'c0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002',
     '2026-02-01T00:00:00+00:00', TRUE, 3, '2026-04-01T00:00:00+00:00'),
    -- B owns the three copies that stay.
    ('as-03', 'c0000000-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003',
     '2026-02-01T00:00:00+00:00', FALSE, 1, NULL),
    ('as-04', 'c0000000-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003',
     '2026-02-01T00:00:00+00:00', FALSE, 0, NULL),
    ('as-05', 'c0000000-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003',
     '2026-02-01T00:00:00+00:00', FALSE, 0, NULL),
    -- C's two copies, no row on the origin: the lower id survives on the origin.
    ('as-06', 'c0000000-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000004',
     '2026-02-01T00:00:00+00:00', FALSE, 1, '2026-03-01T00:00:00+00:00'),
    ('as-07', 'c0000000-0000-4000-8000-000000000006', 'aaaaaaaa-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000004',
     '2026-02-02T00:00:00+00:00', TRUE, 4, '2026-05-01T00:00:00+00:00'),
    -- Usage only: stays a usage row.
    ('as-08', 'a0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000005',
     '2026-01-02T00:00:00+00:00', FALSE, 9, '2026-06-01T00:00:00+00:00'),
    -- An admin assignment: a member.
    ('as-09', 'a0000000-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000001',
     '2026-01-02T00:00:00+00:00', FALSE, 0, NULL),
    -- A favourite: a member.
    ('as-10', 'a0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000006', 'aaaaaaaa-0000-4000-8000-000000000006',
     '2026-01-02T00:00:00+00:00', TRUE, 0, NULL),
    -- The author's own agent: a member.
    ('as-11', 'a0000000-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000007', 'aaaaaaaa-0000-4000-8000-000000000007',
     '2026-01-01T00:00:00+00:00', FALSE, 0, NULL),
    -- Usage with no assigner recorded: not a member.
    ('as-12', 'a0000000-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000006', NULL,
     '2026-01-02T00:00:00+00:00', FALSE, 1, '2026-02-01T00:00:00+00:00'),
    -- B owns the seeder-rewritten copy: their membership lands on the origin.
    ('as-13', 'c0000000-0000-4000-8000-000000000007', 'aaaaaaaa-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003',
     '2026-02-01T00:00:00+00:00', FALSE, 2, NULL),
    -- Someone else only used A's copy: moves to the origin and stays usage.
    ('as-14', 'c0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000007', NULL,
     '2026-02-03T00:00:00+00:00', FALSE, 1, '2026-02-10T00:00:00+00:00'),
    -- B owns the fork of A's copy.
    ('as-15', 'c0000000-0000-4000-8000-000000000008', 'aaaaaaaa-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003',
     '2026-02-05T00:00:00+00:00', FALSE, 0, NULL),
    -- B owns the listed copy.
    ('as-16', 'c0000000-0000-4000-8000-000000000009', 'aaaaaaaa-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003',
     '2026-02-01T00:00:00+00:00', FALSE, 0, NULL),
    -- The operator only used a catalogue agent it owns: not a member.
    ('as-17', 'a0000000-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', NULL,
     '2026-01-03T00:00:00+00:00', FALSE, 2, '2026-01-04T00:00:00+00:00');

INSERT INTO user_agent_preferences (id, user_id, agent_id, is_hidden, created_at) VALUES
    ('pref-1', 'aaaaaaaa-0000-4000-8000-000000000002', 'c0000000-0000-4000-8000-000000000001', TRUE, '2026-02-01T00:00:00+00:00'),
    ('pref-2', 'aaaaaaaa-0000-4000-8000-000000000004', 'c0000000-0000-4000-8000-000000000005', TRUE, '2026-02-01T00:00:00+00:00'),
    ('pref-3', 'aaaaaaaa-0000-4000-8000-000000000004', 'c0000000-0000-4000-8000-000000000006', TRUE, '2026-02-02T00:00:00+00:00');

INSERT INTO store_listings (id, agent_id, tenant_id, publish_status, install_count, created_at, updated_at) VALUES
    ('sl-1', 'a0000000-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'published', 7, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('sl-2', 'a0000000-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000001', 'published', 9, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('sl-3', 'a0000000-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000006', 'published', 3, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    -- The seeder wrote a listing on the copy; the origin has its own, so this one goes with the copy.
    ('sl-4', 'c0000000-0000-4000-8000-000000000007', '11111111-0000-4000-8000-000000000003', 'published', 5, '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    -- The seeder wrote a listing on a copy whose origin has none: it becomes the origin's.
    ('sl-5', 'c0000000-0000-4000-8000-000000000009', '11111111-0000-4000-8000-000000000003', 'published', 9, '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO chat_conversations (id, user_id, tenant_id, title, agent_id, created_at, updated_at) VALUES
    ('conv-a', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000002', 'Tempo', 'c0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    ('conv-a2', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000002', 'Tempo again', 'c0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    ('conv-b', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003', 'Mine', 'c0000000-0000-4000-8000-000000000002', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00'),
    ('conv-c2', 'aaaaaaaa-0000-4000-8000-000000000004', '11111111-0000-4000-8000-000000000005', 'Hills', 'c0000000-0000-4000-8000-000000000006', '2026-02-02T00:00:00+00:00', '2026-02-02T00:00:00+00:00');

INSERT INTO coaching_groups (id, tenant_id, name, agent_id, owner_id, created_at, updated_at) VALUES
    ('22222222-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000002', 'Tempo Crew', 'c0000000-0000-4000-8000-000000000001',
     'aaaaaaaa-0000-4000-8000-000000000002', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO user_facts (id, tenant_id, user_id, scope, kind, predicate_code, object, agent_id, created_at, updated_at) VALUES
    ('fact-a', '11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'user', 'injury', 'have',
     'knee injury', 'c0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO agent_notes (id, tenant_id, user_id, agent_id, scope, content, created_at, updated_at) VALUES
    ('note-a', '11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'c0000000-0000-4000-8000-000000000001',
     'user', 'Prefers mornings.', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO agent_followups (id, tenant_id, user_id, agent_id, content, created_at, updated_at) VALUES
    ('followup-a', '11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'c0000000-0000-4000-8000-000000000001',
     'How did the tempo go?', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO agent_sessions (id, tenant_id, user_id, agent_id, opened_at, created_at, updated_at) VALUES
    ('session-a', '11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'c0000000-0000-4000-8000-000000000001',
     '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00', '2026-02-01T00:00:00+00:00');

INSERT INTO usage_counters (tenant_id, user_id, counter_key, period, agent_id) VALUES
    ('11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'messages', '2026-10', 'c0000000-0000-4000-8000-000000000001');

INSERT INTO agent_introductions (tenant_id, thread_id, agent_id, created_at) VALUES
    ('11111111-0000-4000-8000-000000000002', 'conv-a', 'c0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00'),
    -- The thread already met the origin: that row stays, the copy's goes.
    ('11111111-0000-4000-8000-000000000002', 'conv-a2', 'c0000000-0000-4000-8000-000000000001', '2026-02-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000002', 'conv-a2', 'a0000000-0000-4000-8000-000000000001', '2026-01-15T00:00:00+00:00');
";

/// The agents left: the four origins and the four copies that must stay.
const EXPECTED_AGENTS: [&str; 8] = [
    ORIGIN,
    ORIGIN2,
    USER_PUB,
    ORIGIN3,
    COPY_EDITED,
    COPY_TOUCHED,
    COPY_FOREIGN,
    COPY_OF_COPY,
];

/// Every assignment row afterwards: id, agent, user, membership, use count,
/// favourite, last-used date. `as-02` folded into `as-01`, `as-07` into
/// `as-06` (re-pointed at the origin); `as-13` and `as-14` re-pointed alone,
/// each keeping its own membership.
fn expected_assignments() -> Vec<String> {
    [
        ("as-01", ORIGIN, ATHLETE_A, "member", 5, "fav", "2026-04-01"),
        ("as-03", COPY_EDITED, ATHLETE_B, "member", 1, "-", "-"),
        ("as-04", COPY_TOUCHED, ATHLETE_B, "member", 0, "-", "-"),
        ("as-05", COPY_FOREIGN, ATHLETE_B, "member", 0, "-", "-"),
        (
            "as-06",
            ORIGIN2,
            ATHLETE_C,
            "member",
            5,
            "fav",
            "2026-05-01",
        ),
        ("as-08", ORIGIN, USER_D, "usage", 9, "-", "2026-06-01"),
        ("as-09", ORIGIN2, USER_D, "member", 0, "-", "-"),
        ("as-10", ORIGIN, USER_E, "member", 0, "fav", "-"),
        ("as-11", USER_PUB, AUTHOR, "member", 0, "-", "-"),
        ("as-12", ORIGIN2, USER_E, "usage", 1, "-", "2026-02-01"),
        ("as-13", ORIGIN2, ATHLETE_B, "member", 2, "-", "-"),
        ("as-14", ORIGIN, AUTHOR, "usage", 1, "-", "2026-02-10"),
        ("as-15", COPY_OF_COPY, ATHLETE_B, "member", 0, "-", "-"),
        ("as-16", ORIGIN3, ATHLETE_B, "member", 0, "-", "-"),
        ("as-17", ORIGIN, OPERATOR, "usage", 2, "-", "2026-01-04"),
    ]
    .iter()
    .map(|(id, agent, user, member, uses, fav, last)| {
        format!("{id} {agent} {user} {member} {uses} {fav} {last}")
    })
    .collect()
}

/// Every pointer at an agent the fixture planted, as `table key -> agent`.
fn expected_references() -> Vec<String> {
    let mut rows = vec![
        format!("conversation conv-a -> {ORIGIN}"),
        format!("conversation conv-a2 -> {ORIGIN}"),
        format!("conversation conv-b -> {COPY_EDITED}"),
        format!("conversation conv-c2 -> {ORIGIN2}"),
        format!("followup followup-a -> {ORIGIN}"),
        format!("group Tempo Crew -> {ORIGIN}"),
        format!("intro conv-a -> {ORIGIN}"),
        format!("intro conv-a2 -> {ORIGIN}"),
        format!("note note-a -> {ORIGIN}"),
        format!("pref pref-1 {ATHLETE_A} -> {ORIGIN}"),
        format!("pref pref-2 {ATHLETE_C} -> {ORIGIN2}"),
        format!("selection {A_HOME} {ATHLETE_A} -> {ORIGIN}"),
        format!("selection {B_HOME} {ATHLETE_B} -> {COPY_EDITED}"),
        format!("selection {C_HOME} {ATHLETE_C} -> {ORIGIN2}"),
        format!("selection {C_SECOND} {ATHLETE_C} -> {ORIGIN2}"),
        format!("selection {D_HOME} {USER_D} -> {ORIGIN}"),
        format!("session session-a -> {ORIGIN}"),
        format!("usage messages -> {ORIGIN}"),
        format!("fact fact-a -> {ORIGIN}"),
        format!("fork {COPY_OF_COPY} -> {ORIGIN}"),
        format!("relation rel-1 {USER_PUB} -> {ORIGIN}"),
        format!("relation rel-2 {ORIGIN2} -> {USER_PUB}"),
        format!("listing sl-5 -> {ORIGIN3}"),
    ];
    rows.sort();
    rows
}

/// Install counts afterwards, every listing re-read: members of the agent
/// other than its author, plus the copies forked from it that stayed.
/// `ORIGIN`: A and E, plus the edited, the touched and the re-pointed fork
/// (D and the author only used it). `ORIGIN2`: B, C and D. The
/// athlete-published listing: its author's row is not an install, the
/// foreign copy is. The listing on the seeder-rewritten copy is gone; the one
/// on the listed copy is `ORIGIN3`'s now, counting B.
const EXPECTED_INSTALL_COUNTS: [(&str, i64); 4] =
    [("sl-1", 5), ("sl-2", 3), ("sl-3", 1), ("sl-5", 1)];

/// What the migration left, in a shape both lanes print identically.
#[derive(Debug, PartialEq, Eq)]
struct Landed {
    agents: Vec<String>,
    assignments: Vec<String>,
    references: Vec<String>,
    install_counts: Vec<(String, i64)>,
}

fn assert_landed(landed: &Landed) {
    for moved in [COPY_A, COPY_C1, COPY_C2, COPY_SEEDED, COPY_LISTED] {
        assert!(
            !landed.agents.iter().any(|id| id == moved),
            "{moved} is folded onto its origin and deleted"
        );
    }
    assert_eq!(
        landed.agents,
        EXPECTED_AGENTS.map(str::to_owned).to_vec(),
        "unedited and seeder-rewritten copies of catalogue agents are gone; edited, touched, foreign copies and forks stay"
    );
    assert_eq!(
        landed.assignments,
        expected_assignments(),
        "only real choices are memberships, and each athlete's rows for one origin collapse into one"
    );
    assert_eq!(
        landed.references,
        expected_references(),
        "every reference to a moved copy now names its origin; nothing else moved"
    );
    assert_eq!(
        landed.install_counts,
        EXPECTED_INSTALL_COUNTS
            .iter()
            .map(|(id, n)| ((*id).to_owned(), *n))
            .collect::<Vec<_>>(),
        "every listing's install count is re-read from members plus kept copies"
    );
}

#[tokio::test]
async fn unedited_copies_fold_onto_their_catalogue_agent_and_only_real_choices_become_members() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        migrates_on_postgres(&database.url).await;
        return;
    }
    migrates_on_sqlite(&database.url).await;
}

/// The users every fixture row hangs off.
const USERS: [&str; 7] = [
    OPERATOR, ATHLETE_A, ATHLETE_B, ATHLETE_C, USER_D, USER_E, AUTHOR,
];

/// The reference rows, one text column `r` each; the two lanes differ only in
/// how a UUID column is turned into text, so the `<uuid-text>` slot takes `""` on
/// `SQLite` and `"::text"` on PostgreSQL.
fn reference_queries(u: &str) -> Vec<String> {
    [
        "SELECT 'conversation ' || id || ' -> ' || agent_id AS r FROM chat_conversations WHERE id LIKE 'conv-%'",
        "SELECT 'followup ' || id || ' -> ' || agent_id AS r FROM agent_followups",
        "SELECT 'group ' || name || ' -> ' || agent_id AS r FROM coaching_groups",
        "SELECT 'intro ' || thread_id || ' -> ' || agent_id AS r FROM agent_introductions",
        "SELECT 'note ' || id || ' -> ' || agent_id AS r FROM agent_notes",
        "SELECT 'pref ' || id || ' ' || user_id<uuid-text> || ' -> ' || agent_id AS r FROM user_agent_preferences WHERE id LIKE 'pref-%'",
        "SELECT 'selection ' || tenant_id<uuid-text> || ' ' || user_id<uuid-text> || ' -> ' || selected_agent_id AS r FROM tenant_users WHERE selected_agent_id IS NOT NULL",
        "SELECT 'session ' || id || ' -> ' || agent_id AS r FROM agent_sessions",
        "SELECT 'usage ' || counter_key || ' -> ' || agent_id<uuid-text> AS r FROM usage_counters",
        "SELECT 'fact ' || id || ' -> ' || agent_id AS r FROM user_facts WHERE id LIKE 'fact-%'",
        "SELECT 'fork ' || id || ' -> ' || forked_from AS r FROM agents WHERE id = 'c0000000-0000-4000-8000-000000000008'",
        "SELECT 'relation ' || id || ' ' || agent_id || ' -> ' || related_agent_id AS r FROM agent_relations WHERE id LIKE 'rel-%'",
        "SELECT 'listing ' || id || ' -> ' || agent_id AS r FROM store_listings WHERE id = 'sl-5'",
    ]
    .iter()
    .map(|q| q.replace("<uuid-text>", u))
    .collect()
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. The replay and the fixture
/// run with foreign keys off, as `room_facts_tenant_migration_test` replays
/// them; the migration itself then runs with them on.
async fn sqlite_before_migration(url: &str) -> (sqlx::SqlitePool, String) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for migration in MIGRATOR.iter() {
        if migration.version == TEAM_MEMBERSHIP_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the team-membership migration is in migrations/");
}

async fn migrates_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    for user in USERS {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at, last_active)
             VALUES ($1, $2, 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user)
        .bind(format!("{user}@team-membership.test"))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let landed = sqlite_landed(&pool).await;
    assert_landed(&landed);

    // A second pass (everything after the column it adds, which SQLite cannot
    // add twice) finds nothing left to move.
    let rerun: String = sql
        .lines()
        .filter(|line| !line.starts_with("ALTER TABLE agent_assignments ADD COLUMN is_member"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(rerun, sql, "the re-run skips the ADD COLUMN");
    sqlx::raw_sql(&rerun).execute(&pool).await.unwrap();
    assert_eq!(sqlite_landed(&pool).await, landed, "a re-run moves nothing");
}

async fn sqlite_texts(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query(sql)
        .fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .iter()
        .map(|row| row.get::<String, _>("r"))
        .collect()
}

async fn sqlite_landed(pool: &sqlx::SqlitePool) -> Landed {
    let agents = sqlite_texts(
        pool,
        "SELECT id AS r FROM agents WHERE id LIKE 'a0000000-%' OR id LIKE 'c0000000-%' ORDER BY id",
    )
    .await;
    let assignments = sqlite_texts(
        pool,
        "SELECT id || ' ' || agent_id || ' ' || user_id || ' ' ||
                CASE WHEN is_member THEN 'member' ELSE 'usage' END || ' ' || use_count || ' ' ||
                CASE WHEN is_favorite THEN 'fav' ELSE '-' END || ' ' ||
                COALESCE(substr(last_used_at, 1, 10), '-') AS r
         FROM agent_assignments WHERE id LIKE 'as-%' ORDER BY id",
    )
    .await;
    let mut references = Vec::new();
    for query in reference_queries("") {
        references.extend(sqlite_texts(pool, &query).await);
    }
    references.sort();
    let install_counts = sqlx::query(
        "SELECT id, install_count FROM store_listings WHERE id LIKE 'sl-%' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|row| (row.get("id"), row.get::<i64, _>("install_count")))
    .collect();
    Landed {
        agents,
        assignments,
        references,
        install_counts,
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template, so start again from an empty schema.
#[cfg(feature = "postgresql")]
async fn postgres_before_migration(url: &str) -> (sqlx::PgPool, String) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    for migration in PG_MIGRATOR.iter() {
        if migration.version == TEAM_MEMBERSHIP_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the team-membership migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn migrates_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    for user in USERS {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1::uuid, $2, 'x')")
            .bind(user)
            .bind(format!("{user}@team-membership.test"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let landed = postgres_landed(&pool).await;
    assert_landed(&landed);

    // The PostgreSQL half is idempotent as a whole: a second run moves nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        postgres_landed(&pool).await,
        landed,
        "a re-run moves nothing"
    );
}

#[cfg(feature = "postgresql")]
async fn postgres_texts(pool: &sqlx::PgPool, sql: &str) -> Vec<String> {
    sqlx::query(sql)
        .fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .iter()
        .map(|row| row.get::<String, _>("r"))
        .collect()
}

#[cfg(feature = "postgresql")]
async fn postgres_landed(pool: &sqlx::PgPool) -> Landed {
    let agents = postgres_texts(
        pool,
        "SELECT id AS r FROM agents WHERE id LIKE 'a0000000-%' OR id LIKE 'c0000000-%' \
         ORDER BY id COLLATE \"C\"",
    )
    .await;
    let assignments = postgres_texts(
        pool,
        "SELECT id || ' ' || agent_id || ' ' || user_id::text || ' ' ||
                CASE WHEN is_member THEN 'member' ELSE 'usage' END || ' ' || use_count::text || ' ' ||
                CASE WHEN is_favorite THEN 'fav' ELSE '-' END || ' ' ||
                COALESCE(to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD'), '-') AS r
         FROM agent_assignments WHERE id LIKE 'as-%' ORDER BY id COLLATE \"C\"",
    )
    .await;
    let mut references = Vec::new();
    for query in reference_queries("::text") {
        references.extend(postgres_texts(pool, &query).await);
    }
    references.sort();
    let install_counts = sqlx::query(
        "SELECT id, install_count::bigint AS install_count FROM store_listings \
         WHERE id LIKE 'sl-%' ORDER BY id COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|row| (row.get("id"), row.get::<i64, _>("install_count")))
    .collect();
    Landed {
        agents,
        assignments,
        references,
        install_counts,
    }
}
