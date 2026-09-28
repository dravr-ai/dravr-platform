-- ABOUTME: Drops oauth_apps, whose only writer (TenantRepository::create_oauth_app) had no caller and nothing read it
-- ABOUTME: MCP clients register through oauth2_clients; a tenant's provider apps live in tenant_oauth_credentials
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

DROP TABLE IF EXISTS oauth_apps;
