-- ABOUTME: Drops oauth_apps, whose only writer (TenantRepository::create_oauth_app) had no caller and nothing read it
-- ABOUTME: Also drops authorization_codes, a PostgreSQL-only table no code reads or writes, whose foreign key named oauth_apps
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

DROP TABLE IF EXISTS authorization_codes;
DROP TABLE IF EXISTS oauth_apps;
