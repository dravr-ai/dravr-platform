-- ABOUTME: fuel-product bank — the catalogue of fuel products, their versioned label facts, the review queue and what each athlete carries (PostgreSQL)
-- ABOUTME: Mirrors migrations/20261029200000_fuel_product_bank.sql with native uuid ids, TIMESTAMPTZ and DOUBLE PRECISION (carnet#155)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- fuel_products: one row per product a label names. Global reference data,
-- the same for every tenant, so it is not tenant-scoped by design (the
-- coding-standards exception weather_cache and the mobility catalogue use).
--   gtin         GTIN-14, digits only, zero-padded from UPC-A/EAN-13/EAN-8;
--                unique when present
--   natural_key  brand|name|variant, each trimmed, whitespace-collapsed and
--                lowercased in Rust, so both engines compare the same text
--                (SQLite's lower() folds ASCII only, so the key is never
--                computed in SQL)
CREATE TABLE IF NOT EXISTS fuel_products (
    id UUID PRIMARY KEY,
    gtin TEXT,
    brand TEXT NOT NULL,
    name TEXT NOT NULL,
    variant TEXT NOT NULL DEFAULT '',
    form TEXT NOT NULL CHECK (form IN ('gel', 'drink_mix', 'bar', 'chew', 'capsule', 'tablet', 'other')),
    natural_key TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_fuel_products_gtin
    ON fuel_products (gtin) WHERE gtin IS NOT NULL;

-- fuel_product_versions: the label facts of a product, slowly changing. An
-- approval supersedes the current row (superseded_by points at its
-- successor) and inserts the new one, so a product has exactly one current
-- row. The successor pointer is checked at commit, because the old row is
-- pointed at the new one before the new one is inserted: the partial unique
-- index refuses two current rows at any moment.
--   per-serving facts   energy_kcal .. fluid_ml, NULL when the label omits it
--   per-100g facts      *_per_100g, NULL when the label omits it
--   glucose_fructose_ratio  grams fructose per gram glucose-source (0.8 for
--                       1:0.8); NULL when unknown
--   provenance_json     per-field map: field -> {source, source_url?, confidence?}
--   approved_by_user_id the reviewer; the row outlives their account
CREATE TABLE IF NOT EXISTS fuel_product_versions (
    id UUID PRIMARY KEY,
    product_id UUID NOT NULL REFERENCES fuel_products(id),
    valid_from TIMESTAMPTZ NOT NULL,
    superseded_by UUID REFERENCES fuel_product_versions(id) DEFERRABLE INITIALLY DEFERRED,
    serving_label TEXT,
    serving_g DOUBLE PRECISION,
    energy_kcal DOUBLE PRECISION,
    carbs_g DOUBLE PRECISION,
    sugars_g DOUBLE PRECISION,
    protein_g DOUBLE PRECISION,
    fat_g DOUBLE PRECISION,
    sodium_mg DOUBLE PRECISION,
    potassium_mg DOUBLE PRECISION,
    caffeine_mg DOUBLE PRECISION,
    fluid_ml DOUBLE PRECISION,
    carbs_g_per_100g DOUBLE PRECISION,
    sugars_g_per_100g DOUBLE PRECISION,
    protein_g_per_100g DOUBLE PRECISION,
    fat_g_per_100g DOUBLE PRECISION,
    sodium_mg_per_100g DOUBLE PRECISION,
    salt_g_per_100g DOUBLE PRECISION,
    glucose_fructose_ratio DOUBLE PRECISION,
    provenance_json TEXT NOT NULL,
    approved_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    approved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_fuel_product_versions_current
    ON fuel_product_versions (product_id) WHERE superseded_by IS NULL;

-- fuel_product_proposals: the review queue. Nothing here reaches the
-- catalogue until an operator approves it; global like the catalogue.
--   product_id       set when the proposal updates an existing product, and
--                    on approval to the product it landed on
--   source_image     the label panel the facts were read from, served by the
--                    backend to the reviewer
--   source_hash      sha256 hex of the image bytes and URL; one proposal per
--                    source
--   facts_json       the proposed facts, same field names as the versions
--   gate_flags_json  the plausibility-gate codes the proposal tripped
--   reject_reason    a code the admin console validates
CREATE TABLE IF NOT EXISTS fuel_product_proposals (
    id UUID PRIMARY KEY,
    product_id UUID REFERENCES fuel_products(id),
    brand TEXT NOT NULL,
    name TEXT NOT NULL,
    variant TEXT NOT NULL DEFAULT '',
    form TEXT NOT NULL CHECK (form IN ('gel', 'drink_mix', 'bar', 'chew', 'capsule', 'tablet', 'other')),
    gtin TEXT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('shopify_ocr', 'athlete', 'admin')),
    source_url TEXT,
    source_image BYTEA,
    source_image_mime TEXT,
    source_hash TEXT,
    facts_json TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    gate_flags_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    proposed_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    reviewed_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    reviewed_at TIMESTAMPTZ,
    reject_reason TEXT CHECK (reject_reason IN ('illegible', 'wrong_product', 'implausible', 'duplicate', 'not_fuel', 'other')),
    reject_note TEXT,
    approved_version_id UUID REFERENCES fuel_product_versions(id),
    created_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_fuel_product_proposals_source_hash
    ON fuel_product_proposals (source_hash) WHERE source_hash IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_fuel_product_proposals_queue
    ON fuel_product_proposals (status, created_at);

-- athlete_fuel_items: what an athlete carries, in one tenant. A catalogue
-- product, or a pending proposal until it is approved, when the approval
-- sets product_id and keeps proposal_id. Removal keeps the row.
CREATE TABLE IF NOT EXISTS athlete_fuel_items (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    product_id UUID REFERENCES fuel_products(id),
    proposal_id UUID REFERENCES fuel_product_proposals(id),
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    removed_at TIMESTAMPTZ,
    CHECK (product_id IS NOT NULL OR proposal_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_athlete_fuel_items_owner
    ON athlete_fuel_items (tenant_id, user_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_athlete_fuel_items_product
    ON athlete_fuel_items (tenant_id, user_id, product_id)
    WHERE removed_at IS NULL AND product_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_athlete_fuel_items_pending
    ON athlete_fuel_items (tenant_id, user_id, proposal_id)
    WHERE removed_at IS NULL AND product_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_athlete_fuel_items_proposal
    ON athlete_fuel_items (proposal_id) WHERE proposal_id IS NOT NULL;
