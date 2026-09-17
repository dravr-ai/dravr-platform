-- ABOUTME: Rewrites machine-stamped conversation titles ("Messaging: telegram", "Chat Sep 16 14:03") to the room's or the agent's name (PostgreSQL)
-- ABOUTME: The forge and the REST create now stamp that name; this repairs the rows that predate it so no client needs the old regex

-- Two stamps nobody typed: the messaging forge's "Messaging: <channel>" and
-- the dated in-app title ("Chat Sep 16 14:03", "Discussion 16 sept. 14:03",
-- "Conversa 16 de set. 14:03", plus the 12-hour form older clients wrote).
-- The rule that replaces them is the forge's: room name, else agent title.
-- A dated title on a thread with no agent is left alone — it is still the
-- only thing that tells two such threads apart.

-- 1. A row with a room takes the room's name.
UPDATE chat_conversations c
SET title = g.name
FROM coaching_groups g
WHERE g.id = c.group_id
  AND c.title LIKE 'Messaging: %';

-- 2. A 1:1 row with an agent takes the agent's title.
UPDATE chat_conversations c
SET title = a.title
FROM agents a
WHERE a.id = c.agent_id
  AND c.group_id IS NULL
  AND (
       c.title LIKE 'Messaging: %'
    OR c.title ~ '^(Chat|Discussion|Conversa) .+ [0-9]{1,2}:[0-9]{2}( [AP]M)?$'
  );

-- 3. Whatever still reads "Messaging: <channel>" has neither a room nor an
--    agent that exists: it becomes the channel's name, capitalised, so no row
--    prints the machine prefix once the client regex is gone.
UPDATE chat_conversations
SET title = upper(substr(title, 12, 1)) || substr(title, 13)
WHERE title LIKE 'Messaging: %';
