-- data_sources identity must treat NULL device metadata as equal
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The table-level UNIQUE(user_id, tenant_id, provider, device_model, source)
-- treats NULL device_model/source as distinct, so the provider-level upsert
-- (which carries no device metadata) would insert a fresh row on every sync
-- instead of conflicting. The NULL-coalesced unique index below is what the
-- upsert's ON CONFLICT clause targets. SQLite cannot drop a table-level
-- UNIQUE without a rebuild; the old constraint stays but never fires for
-- NULL device metadata, so this index is the effective identity.
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_sources_identity
    ON data_sources (user_id, tenant_id, provider, COALESCE(device_model, ''), COALESCE(source, ''));
