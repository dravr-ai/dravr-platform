-- ABOUTME: Re-shape subscriptions table for the pluggable BillingProvider trait
-- ABOUTME: Drops + recreates with provider_* columns; safe because Phase 5 had not shipped real subscriptions

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

DROP TABLE IF EXISTS subscriptions;

CREATE TABLE subscriptions (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    provider TEXT NOT NULL,
    provider_customer_id TEXT NOT NULL,
    provider_subscription_id TEXT,
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
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider, provider_customer_id),
    UNIQUE (provider, provider_subscription_id)
);

CREATE INDEX idx_subscriptions_tenant ON subscriptions(tenant_id);
CREATE INDEX idx_subscriptions_user ON subscriptions(user_id);
CREATE INDEX idx_subscriptions_status ON subscriptions(status);
CREATE INDEX idx_subscriptions_period_end ON subscriptions(current_period_end);
