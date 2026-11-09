-- ABOUTME: Drops a2a_client_api_keys and a2a_clients.api_key_hash: an A2A client's only way in is OAuth (SQLite)
-- ABOUTME: Mirrors migrations_pg/20261108120000; rebuilds a2a_clients, keeping every client, session and usage row

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An A2A client authenticates with its OAuth registration only: the
-- client_credentials grant or an authorization code, as its A2A 1.0.1 agent
-- card states. Registration stopped issuing API keys and Ed25519 keypairs, so
-- the link table naming a client's API keys and the column that held the
-- keypair's public key (misnamed api_key_hash) have no writer and no reader.
--
-- A key the link table still names belonged to a client registered before
-- that, and authenticates until it is retired: it is retired first, while
-- the link still says which keys are a client's.
UPDATE api_keys SET is_active = 0
WHERE id IN (SELECT api_key_id FROM a2a_client_api_keys);

DROP TABLE IF EXISTS a2a_client_api_keys;

-- SQLite refuses to drop a UNIQUE column, so a2a_clients is rebuilt without
-- it. sqlx applies a migration inside a transaction with foreign keys ON,
-- where PRAGMA foreign_keys cannot be turned off, and dropping the old table
-- deletes its rows first, cascading into a2a_sessions and a2a_usage. Both
-- are kept aside and restored once the rebuilt table holds every client.
-- a2a_tasks and a2a_push_notification_configs reference neither table.
CREATE TEMP TABLE IF NOT EXISTS a2a_sessions_kept AS SELECT * FROM a2a_sessions;
CREATE TEMP TABLE IF NOT EXISTS a2a_usage_kept AS SELECT * FROM a2a_usage;

CREATE TABLE IF NOT EXISTS a2a_clients_next (
    client_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    capabilities TEXT NOT NULL DEFAULT '[]',
    redirect_uris TEXT NOT NULL DEFAULT '[]',
    contact_email TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    rate_limit_per_minute INTEGER NOT NULL DEFAULT 100,
    rate_limit_per_day INTEGER NOT NULL DEFAULT 10000,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO a2a_clients_next
    (client_id, user_id, name, description, capabilities, redirect_uris, contact_email,
     is_active, rate_limit_per_minute, rate_limit_per_day, created_at, updated_at)
SELECT client_id, user_id, name, description, capabilities, redirect_uris, contact_email,
       is_active, rate_limit_per_minute, rate_limit_per_day, created_at, updated_at
FROM a2a_clients;

DROP TABLE IF EXISTS a2a_clients;

ALTER TABLE a2a_clients_next RENAME TO a2a_clients;

INSERT INTO a2a_sessions SELECT * FROM temp.a2a_sessions_kept;
INSERT INTO a2a_usage SELECT * FROM temp.a2a_usage_kept;

DROP TABLE IF EXISTS temp.a2a_sessions_kept;
DROP TABLE IF EXISTS temp.a2a_usage_kept;
