-- ABOUTME: memory_extraction_jobs.usage_tenant_id — the tenant an owed extraction's llm_usage row is billed under
-- ABOUTME: The conversation's tenant, like every other row of the turn; tenant_id stays the tenant its facts are stamped under

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#104. A guided answer in a shared room stamps its facts under the
-- athlete's own tenant while the room's conversation, and every other usage row
-- of that turn, sits under the channel tenant. The extraction's usage row
-- followed the facts, so the turn's cost was split across two tenants. A column
-- rather than a payload field, for the reason tenant_id is one: TenantId is
-- deliberately not deserialisable. NULL on a row recorded before this column,
-- which bills under tenant_id as it always did.
ALTER TABLE memory_extraction_jobs ADD COLUMN IF NOT EXISTS usage_tenant_id UUID;
