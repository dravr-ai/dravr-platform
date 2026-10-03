-- ABOUTME: Clears the selected agent of every account that answered it coaches others and does not train (PostgreSQL)
-- ABOUTME: Data only: the starter agent signup picked for them before that answer was ever theirs

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Signup selects the tenant's first system agent for every new account, before
-- the account says whether it trains. A coach who does not train is recorded
-- as an athlete step (`about_you` or `parq`) marked `not_applicable`, and from
-- now on that answer clears the selection as it is written. This catches the
-- accounts that answered before it did. `user_onboarding.user_id` is TEXT and
-- `tenant_users.user_id` is UUID, so the comparison is on the text form.
UPDATE tenant_users
SET selected_agent_id = NULL
WHERE selected_agent_id IS NOT NULL
  AND user_id::text IN (
      SELECT user_id
      FROM user_onboarding
      WHERE step_id IN ('about_you', 'parq')
        AND status = 'not_applicable'
  );
