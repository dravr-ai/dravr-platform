-- ABOUTME: An athlete's all-time best effort at each standard running distance, and the activities already scanned for one
-- ABOUTME: A synced run faster than the stored best at 5 km, 10 km, half or full marathon is a new personal record
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per (athlete, tenant, distance): the fastest elapsed time any scanned
-- run covered that distance in, and the run that set it. distance is the
-- catalogue code ('5k', '10k', 'half_marathon', 'marathon'); provider and
-- activity_id name the run, so a provider's disconnect purge reaches the row.
CREATE TABLE IF NOT EXISTS personal_best_efforts (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    distance TEXT NOT NULL,
    elapsed_seconds REAL NOT NULL,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    achieved_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, tenant_id, distance)
);

-- One row per activity whose best efforts were computed, so each run costs its
-- one streams fetch once, however many syncs list it again. An athlete with no
-- row here has never been scanned: the first scan seeds the bests silently.
CREATE TABLE IF NOT EXISTS best_effort_scans (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    scanned_at TEXT NOT NULL,
    PRIMARY KEY (user_id, tenant_id, provider, activity_id)
);
