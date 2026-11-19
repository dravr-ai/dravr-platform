-- ABOUTME: rotated_to_token_id on admin_tokens: the replacement a rotation minted, written by the rotation's claim (SQLite)
-- ABOUTME: Tells a rotated token from a revoked one, so revoking a rotated token can name the credential that is still live
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A rotation retires a token and mints its replacement in one transaction.
-- The retired row and a revoked row were both `is_active = FALSE` and
-- nothing linked a row to its replacement, so revoking an already-rotated
-- token answered success while the replacement stayed live (carnet#1589).
-- The rotation's claim now writes the replacement's id here, and a revoke of
-- a token that names one answers 409 with that id.
--
-- NULL for a live token, for a revoked one, and for every token rotated
-- before this column: those rotations cannot be told from a revocation and
-- keep the answer a revoked token gets. TEXT with no foreign key, like
-- admin_tokens.tenant_id and admin_tokens.operator_user_id.

ALTER TABLE admin_tokens ADD COLUMN rotated_to_token_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
