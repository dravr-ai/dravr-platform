-- ABOUTME: Extract store listing fields from coaches into dedicated store_listings table
-- ABOUTME: Part of Coach struct decomposition (DRAVR-592)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Create the store_listings table for Store publishing workflow
CREATE TABLE IF NOT EXISTS store_listings (
    id TEXT PRIMARY KEY,
    coach_id TEXT NOT NULL UNIQUE REFERENCES coaches(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    publish_status TEXT NOT NULL DEFAULT 'draft',
    published_at TEXT,
    review_submitted_at TEXT,
    review_decision_at TEXT,
    review_decision_by TEXT,
    rejection_reason TEXT,
    install_count INTEGER NOT NULL DEFAULT 0,
    icon_url TEXT,
    author_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Indexes for efficient store queries
CREATE INDEX IF NOT EXISTS idx_store_listings_coach_id ON store_listings(coach_id);
CREATE INDEX IF NOT EXISTS idx_store_listings_status ON store_listings(publish_status);
CREATE INDEX IF NOT EXISTS idx_store_listings_tenant ON store_listings(tenant_id);
CREATE INDEX IF NOT EXISTS idx_store_listings_published_at ON store_listings(published_at);

-- Copy existing store data from coaches to store_listings
-- Only copy coaches that have non-default store state (submitted, published, or rejected)
INSERT INTO store_listings (id, coach_id, tenant_id, publish_status, published_at,
    review_submitted_at, review_decision_at, review_decision_by, rejection_reason,
    install_count, icon_url, author_id, created_at, updated_at)
SELECT
    lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' ||
          substr(hex(randomblob(2)),2) || '-' ||
          substr('89ab', abs(random()) % 4 + 1, 1) ||
          substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
    id,
    tenant_id,
    COALESCE(publish_status, 'draft'),
    published_at,
    review_submitted_at,
    review_decision_at,
    review_decision_by,
    rejection_reason,
    COALESCE(install_count, 0),
    icon_url,
    author_id,
    COALESCE(created_at, datetime('now')),
    COALESCE(updated_at, datetime('now'))
FROM coaches
WHERE publish_status IS NOT NULL AND publish_status != 'draft';

-- Drop indexes that reference store columns before dropping them
-- SQLite requires indexes to be removed before the columns they reference
DROP INDEX IF EXISTS idx_coaches_publish_status;
DROP INDEX IF EXISTS idx_coaches_published;
DROP INDEX IF EXISTS idx_coaches_pending_review;
DROP INDEX IF EXISTS idx_coaches_install_count;
DROP INDEX IF EXISTS idx_coaches_author;

-- Drop store columns from coaches table
-- SQLite 3.35.0+ supports ALTER TABLE DROP COLUMN
ALTER TABLE coaches DROP COLUMN publish_status;
ALTER TABLE coaches DROP COLUMN published_at;
ALTER TABLE coaches DROP COLUMN review_submitted_at;
ALTER TABLE coaches DROP COLUMN review_decision_at;
ALTER TABLE coaches DROP COLUMN review_decision_by;
ALTER TABLE coaches DROP COLUMN rejection_reason;
ALTER TABLE coaches DROP COLUMN install_count;
ALTER TABLE coaches DROP COLUMN icon_url;
ALTER TABLE coaches DROP COLUMN author_id;
