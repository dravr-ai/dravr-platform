-- ABOUTME: Promotes coaches.slug to the addressable catalogue handle (@handle).
-- ABOUTME: One origin coach owns a handle; installed copies carry it as a reference.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The catalogue is cross-tenant (store listings are browsed and installed
-- across tenants), so the handle is unique across the whole table, not per
-- tenant as idx_coaches_slug had it. Only origin coaches (forked_from IS NULL)
-- own a handle; a user's installed copy carries the origin's handle so the
-- copy resolves by the same name, and is deliberately outside the index.
DROP INDEX IF EXISTS idx_coaches_slug;

CREATE UNIQUE INDEX IF NOT EXISTS idx_coaches_handle
    ON coaches(slug)
    WHERE slug IS NOT NULL AND forked_from IS NULL;

-- Copies installed before handles existed inherit their origin's handle.
UPDATE coaches
SET slug = (SELECT origin.slug FROM coaches origin WHERE origin.id = coaches.forked_from)
WHERE forked_from IS NOT NULL AND slug IS NULL;

-- Resolving an installed coach by handle for a user.
CREATE INDEX IF NOT EXISTS idx_coaches_handle_copy
    ON coaches(slug, user_id)
    WHERE slug IS NOT NULL AND forked_from IS NOT NULL;
