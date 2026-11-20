-- ABOUTME: deletion_receipt_anchors — where the deletion receipt chain's head was anchored: a signed record_log checkpoint
-- ABOUTME: Written by the record_log worker after each checkpoint it cuts; read by the admin ledger and pierre-cli

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The receipt chain proves no receipt was altered or removed from the
-- middle; its head must be recorded somewhere the database cannot rewrite.
-- Each signed checkpoint (barrage record_log, carnet#1542) holds the head as
-- its leaf 0 and is logged to Cloud Logging; this row says which receipt that
-- was, under which signed tree. The table is global, as the receipts are.
CREATE TABLE IF NOT EXISTS deletion_receipt_anchors (
    receipt_seq    BIGINT  NOT NULL,
    receipt_hash   TEXT    NOT NULL,
    checkpoint_id  BIGINT  NOT NULL PRIMARY KEY,
    tree_size      BIGINT  NOT NULL,
    tree_root      TEXT    NOT NULL,
    note           TEXT    NOT NULL,
    anchored_at    TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_deletion_receipt_anchors_seq
    ON deletion_receipt_anchors (receipt_seq);
