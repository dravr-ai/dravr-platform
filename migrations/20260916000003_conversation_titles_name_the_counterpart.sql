-- ABOUTME: Rewrites machine-stamped conversation titles ("Messaging: telegram", "Chat Sep 16 14:03") to the room's or the agent's name
-- ABOUTME: The forge and the REST create now stamp that name; this repairs the rows that predate it so no client needs the old regex

-- Two stamps nobody typed: the messaging forge's "Messaging: <channel>" and
-- the dated in-app title ("Chat Sep 16 14:03", "Discussion 16 sept. 14:03",
-- "Conversa 16 de set. 14:03", plus the 12-hour form older clients wrote).
-- The rule that replaces them is the forge's: room name, else agent title.
-- A dated title on a thread with no agent is left alone — it is still the
-- only thing that tells two such threads apart.

-- 1. A row with a room takes the room's name.
UPDATE chat_conversations
SET title = (SELECT g.name FROM coaching_groups g WHERE g.id = chat_conversations.group_id)
WHERE group_id IS NOT NULL
  AND title LIKE 'Messaging: %'
  AND EXISTS (SELECT 1 FROM coaching_groups g WHERE g.id = chat_conversations.group_id);

-- 2. A 1:1 row with an agent takes the agent's title.
UPDATE chat_conversations
SET title = (SELECT a.title FROM agents a WHERE a.id = chat_conversations.agent_id)
WHERE agent_id IS NOT NULL
  AND group_id IS NULL
  AND (
       title LIKE 'Messaging: %'
    OR title GLOB 'Chat * [0-9][0-9]:[0-9][0-9]'
    OR title GLOB 'Chat * [0-9]:[0-9][0-9] [AP]M'
    OR title GLOB 'Chat * [0-9][0-9]:[0-9][0-9] [AP]M'
    OR title GLOB 'Discussion * [0-9][0-9]:[0-9][0-9]'
    OR title GLOB 'Discussion * [0-9]:[0-9][0-9] [AP]M'
    OR title GLOB 'Discussion * [0-9][0-9]:[0-9][0-9] [AP]M'
    OR title GLOB 'Conversa * [0-9][0-9]:[0-9][0-9]'
    OR title GLOB 'Conversa * [0-9]:[0-9][0-9] [AP]M'
    OR title GLOB 'Conversa * [0-9][0-9]:[0-9][0-9] [AP]M'
  )
  AND EXISTS (SELECT 1 FROM agents a WHERE a.id = chat_conversations.agent_id);

-- 3. Whatever still reads "Messaging: <channel>" has neither a room nor an
--    agent that exists: it becomes the channel's name, capitalised, so no row
--    prints the machine prefix once the client regex is gone.
UPDATE chat_conversations
SET title = upper(substr(title, 12, 1)) || substr(title, 13)
WHERE title LIKE 'Messaging: %';
