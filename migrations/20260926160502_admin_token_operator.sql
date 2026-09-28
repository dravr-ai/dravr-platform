-- ABOUTME: operator_user_id on admin_tokens — the super-admin a device-login token acts as, stored at the grant (SQLite)
-- ABOUTME: Replaces reading the operator out of the free-text service_name, and takes operator emails out of token names

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A device login minted a super-admin token named `device-cli:<approver
-- email>`, and the config routes and the pre-approval audit resolved the
-- operator from that name. Any ManageAdminTokens holder could create a token
-- with the same name and have its writes attributed to another operator, and
-- every handler that logs the token's service name logged the operator's
-- email (carnet#561). The operator is now its own column, written only by the
-- device grant; the name is a label that carries the operator's id.
ALTER TABLE admin_tokens ADD COLUMN operator_user_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Device-login tokens minted before this column: resolve the approver's
-- email to their account.
UPDATE admin_tokens
SET operator_user_id = (
    SELECT u.id FROM users u WHERE u.email = substr(admin_tokens.service_name, 12)
)
WHERE service_name LIKE 'device-cli:%';

-- And drop the email from every such name: the resolved id where there is
-- one, a label naming no one where the approver's account is gone.
UPDATE admin_tokens
SET service_name = 'device-cli:' || COALESCE(operator_user_id, 'unresolved')
WHERE service_name LIKE 'device-cli:%';
