-- ABOUTME: Records when the provider event last applied to a subscription happened, so an older event never overwrites a newer one (carnet#993)
-- ABOUTME: Additive only: one nullable column; a row written before it accepts the next event whatever its time

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Stripe neither orders webhook deliveries nor stops retrying a failed one
-- for three days, so a customer.subscription.updated sent before a
-- customer.subscription.deleted can land after it and re-activate the
-- canceled row. The upsert now refuses an event older than the one the row
-- last applied, read from this column (the event's own `created` time).
ALTER TABLE subscriptions ADD COLUMN IF NOT EXISTS last_event_at TIMESTAMPTZ;
