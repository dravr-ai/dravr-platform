-- data_sources identity must treat NULL device metadata as equal
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The table-level UNIQUE(user_id, tenant_id, provider, device_model, source)
-- treats NULL device_model/source as distinct, so the provider-level upsert
-- (which carries no device metadata) would insert a fresh row on every sync
-- instead of conflicting. Replace it with a NULL-coalesced unique expression
-- index; the upsert's ON CONFLICT clause targets these expressions.
DO $$
DECLARE cname text;
BEGIN
    SELECT conname INTO cname FROM pg_constraint
    WHERE conrelid = 'data_sources'::regclass AND contype = 'u';
    IF cname IS NOT NULL THEN
        EXECUTE format('ALTER TABLE data_sources DROP CONSTRAINT %I', cname);
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_data_sources_identity
    ON data_sources (user_id, tenant_id, provider, COALESCE(device_model, ''), COALESCE(source, ''));
