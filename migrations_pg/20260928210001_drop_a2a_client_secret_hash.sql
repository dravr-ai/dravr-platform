-- ABOUTME: Drops a2a_clients.client_secret_hash, a SHA-256 of the registration secret that nothing ever read
-- ABOUTME: An A2A client's secret lives in oauth2_clients (Argon2), where the /oauth2/token client_credentials grant verifies it
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE a2a_clients DROP COLUMN IF EXISTS client_secret_hash;
