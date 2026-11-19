-- ABOUTME: Records the transport each coach read of an athlete's data was served over (PostgreSQL)
-- ABOUTME: Adds the nullable coach_data_reads.transport, so a read served to an outside assistant is marked (carnet#1660)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The transport the read was served over, by its stable label
-- (pierre_core::transport::Transport::as_str): 'mcp_http', 'a2a' or
-- 'api_key' when the coach's outside assistant received the data, which a
-- company outside Dravr runs; 'web_app', 'mobile_app', 'messaging' or
-- 'platform_job' for Dravr's own surfaces. 'undeclared' for a read that
-- declared no transport (pierre_core::transport::UNDECLARED_TRANSPORT). NULL
-- only for the reads recorded before this column: those are not guessed
-- (EFVP 2026-11-17, decision 9.4b). Additive and nullable; no backfill.
ALTER TABLE coach_data_reads ADD COLUMN IF NOT EXISTS transport TEXT;
