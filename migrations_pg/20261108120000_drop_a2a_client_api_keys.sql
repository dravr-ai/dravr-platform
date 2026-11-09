-- ABOUTME: Drops a2a_client_api_keys and a2a_clients.api_key_hash: an A2A client's only way in is OAuth
-- ABOUTME: Retires every API key the link table still names first, so a dropped link leaves no working key

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
-- the link still says which keys are a client's. The guard lets a re-run,
-- after the link is gone, retire nothing and drop nothing.
DO $$
BEGIN
    IF to_regclass('a2a_client_api_keys') IS NOT NULL THEN
        UPDATE api_keys SET is_active = FALSE
        WHERE id IN (SELECT api_key_id FROM a2a_client_api_keys);
    END IF;
END
$$;

DROP TABLE IF EXISTS a2a_client_api_keys;

ALTER TABLE a2a_clients DROP COLUMN IF EXISTS api_key_hash;
