-- ABOUTME: Phase 5 — subscriptions table for Stripe-backed per-tenant billing
-- ABOUTME: One row per (tenant_id, stripe_subscription_id); user_id is the upgrader/owner
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS subscriptions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    stripe_customer_id TEXT NOT NULL UNIQUE,
    stripe_subscription_id TEXT UNIQUE,
    status TEXT NOT NULL CHECK (status IN (
        'trialing', 'active', 'past_due', 'canceled',
        'incomplete', 'incomplete_expired', 'unpaid'
    )),
    plan_tier TEXT NOT NULL CHECK (plan_tier IN ('starter', 'professional', 'enterprise')),
    current_period_start TEXT,
    current_period_end TEXT,
    cancel_at_period_end INTEGER NOT NULL DEFAULT 0,
    canceled_at TEXT,
    trial_end TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_subscriptions_tenant ON subscriptions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_subscriptions_status ON subscriptions(status);
CREATE INDEX IF NOT EXISTS idx_subscriptions_period_end ON subscriptions(current_period_end);
