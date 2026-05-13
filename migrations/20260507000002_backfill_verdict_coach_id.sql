-- ABOUTME: Backfill claim_verdicts.coach_id from chat_conversations.coach_id and users.default_coach_id
-- ABOUTME: Pre-fix messaging conversations were created with coach_id=NULL so verdicts couldn't attribute

-- Step 1: Pull coach_id forward from the conversation when it exists.
UPDATE claim_verdicts
SET coach_id = (
    SELECT cc.coach_id
    FROM chat_conversations AS cc
    WHERE cc.id = claim_verdicts.conversation_id
)
WHERE coach_id IS NULL
  AND conversation_id IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM chat_conversations AS cc
      WHERE cc.id = claim_verdicts.conversation_id
        AND cc.coach_id IS NOT NULL
  );

-- Step 2: For verdicts whose conversation also has no coach_id, fall back to
-- the user's default_coach_id. This covers messaging traffic where the
-- conversation was created before the messaging-ingress fix that propagates
-- the user's default coach into the chat_conversations row.
UPDATE claim_verdicts
SET coach_id = (
    SELECT u.default_coach_id
    FROM users AS u
    WHERE u.id = claim_verdicts.user_id
)
WHERE coach_id IS NULL
  AND EXISTS (
      SELECT 1 FROM users AS u
      WHERE u.id = claim_verdicts.user_id
        AND u.default_coach_id IS NOT NULL
  );

-- Step 3: Backfill chat_conversations.coach_id from users.default_coach_id
-- so going forward, the conversation row carries the canonical coach.
UPDATE chat_conversations
SET coach_id = (
    SELECT u.default_coach_id
    FROM users AS u
    WHERE u.id = chat_conversations.user_id
)
WHERE coach_id IS NULL
  AND EXISTS (
      SELECT 1 FROM users AS u
      WHERE u.id = chat_conversations.user_id
        AND u.default_coach_id IS NOT NULL
  );
