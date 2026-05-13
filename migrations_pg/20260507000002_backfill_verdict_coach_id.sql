-- ABOUTME: Backfill claim_verdicts.coach_id from chat_conversations.coach_id and users.default_coach_id
-- ABOUTME: Pre-fix messaging conversations were created with coach_id=NULL so verdicts couldn't attribute

UPDATE claim_verdicts cv
SET coach_id = cc.coach_id
FROM chat_conversations cc
WHERE cv.conversation_id = cc.id
  AND cv.coach_id IS NULL
  AND cc.coach_id IS NOT NULL;

-- claim_verdicts.user_id is VARCHAR(255) storing UUID strings; users.id is
-- UUID. PG refuses the implicit comparison, so cast the UUID side to text.
UPDATE claim_verdicts cv
SET coach_id = u.default_coach_id
FROM users u
WHERE cv.user_id = u.id::text
  AND cv.coach_id IS NULL
  AND u.default_coach_id IS NOT NULL;

UPDATE chat_conversations cc
SET coach_id = u.default_coach_id
FROM users u
WHERE cc.user_id = u.id
  AND cc.coach_id IS NULL
  AND u.default_coach_id IS NOT NULL;
