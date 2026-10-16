-- ABOUTME: Records when the provider event last applied to a subscription happened, so an older event never overwrites a newer one (carnet#993)
-- ABOUTME: Additive only: one nullable column; a row written before it accepts the next event whatever its time

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Stripe neither orders webhook deliveries nor stops retrying a failed one
-- for three days, so a customer.subscription.updated sent before a
-- customer.subscription.deleted can land after it and re-activate the
-- canceled row. The upsert now refuses an event older than the one the row
-- last applied, read from this column (the event's own `created` time, whole
-- seconds, written as the same RFC 3339 text every time so the text compares
-- in time order).
ALTER TABLE subscriptions ADD COLUMN last_event_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
