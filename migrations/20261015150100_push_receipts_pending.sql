-- ABOUTME: Persists every push Expo accepted until its receipt is read, so a receipt outlives the instance that sent it (carnet#984)
-- ABOUTME: Additive only: one new table dravr-commere 0.6.0's scheduler sweeps once a receipt is 15 minutes old

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- dravr-commere read a push's receipt from a 15-minute in-memory timer, which
-- a scale-to-zero instance or a deploy dropped, so a device the receipt
-- reported unregistered was never retired. 0.6.0 records each accepted ticket
-- here; its scheduler tick reads the due rows, retires the devices whose
-- receipt says DeviceNotRegistered, and deletes what it read plus anything
-- past Expo's 24-hour receipt retention. Shape and column types are the ones
-- dravr-commere's sqlite repository binds.
CREATE TABLE IF NOT EXISTS push_receipts_pending (
    ticket_id       TEXT NOT NULL PRIMARY KEY,
    device_token_id TEXT NOT NULL,
    notification_id TEXT NOT NULL,
    user_id         TEXT NOT NULL,
    tenant_id       TEXT NOT NULL,
    sent_at         TEXT NOT NULL
);
