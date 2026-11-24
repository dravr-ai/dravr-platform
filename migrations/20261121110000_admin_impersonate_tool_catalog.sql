-- ABOUTME: Seeds the tool_catalog rows for admin_impersonate_start and admin_impersonate_end (carnet#1511)
-- ABOUTME: Catalogued like the other admin tools, so a tenant can disable them and the catalog completeness test holds
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The two admin support-session tools register in the `admin` category
-- (ADMIN_ONLY). An uncatalogued tool is always enabled and cannot be disabled
-- per tenant (carnet#143). Same shape as the other admin rows (tc-062):
-- enabled by default, `starter`, requires_provider NULL. tc-158 and tc-159
-- are the next ids after tc-157, the highest any migration claims.
-- INSERT OR IGNORE: a database that already booted holds a synthesised
-- `tc-auto-` row under each name and keeps it, with any text an admin edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-158', 'admin_impersonate_start', 'Start Support Session', 'Act as one user through this connection for up to an hour, every call audited (admin only)', 'admin', 1, NULL, 'starter'),
('tc-159', 'admin_impersonate_end', 'End Support Session', 'End the support session this connection started (admin only)', 'admin', 1, NULL, 'starter');
