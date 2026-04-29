-- ABOUTME: Phase 5 — subscriptions table for Stripe-backed per-tenant billing
-- ABOUTME: One row per (tenant_id, stripe_subscription_id); user_id is the upgrader/owner
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS subscriptions (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    stripe_customer_id TEXT NOT NULL UNIQUE,
    stripe_subscription_id TEXT UNIQUE,
    status TEXT NOT NULL CHECK (status IN (
        'trialing', 'active', 'past_due', 'canceled',
        'incomplete', 'incomplete_expired', 'unpaid'
    )),
    plan_tier TEXT NOT NULL CHECK (plan_tier IN ('starter', 'professional', 'enterprise')),
    current_period_start TIMESTAMPTZ,
    current_period_end TIMESTAMPTZ,
    cancel_at_period_end BOOLEAN NOT NULL DEFAULT FALSE,
    canceled_at TIMESTAMPTZ,
    trial_end TIMESTAMPTZ,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_subscriptions_tenant ON subscriptions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_subscriptions_status ON subscriptions(status);
CREATE INDEX IF NOT EXISTS idx_subscriptions_period_end ON subscriptions(current_period_end);
