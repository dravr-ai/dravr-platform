-- ABOUTME: Re-expresses persisted command replies from canot's rich-text tags to inline markdown
-- ABOUTME: One-off data fix; every command reply written after it is authored in markdown at the source

-- Command handlers used to answer in canot's HTML subset (<b>, <i>, <code>),
-- and a reply persisted before the pipeline converted it for the in-app
-- surfaces still carries those tags. The catalogue rows are now written in
-- inline markdown, the messaging egress translates markdown into the channel
-- dialect, and the clients no longer repair a row on read — so the rows that
-- were persisted tagged are rewritten once, here.
--
-- Scoped to command replies: coach prose never carried the dialect, and a
-- literal `<b>` an athlete typed must stay theirs. The entity decodes undo
-- the escaping the dialect applied to text nodes in those same rows.
UPDATE chat_messages
SET content = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(
    content,
    '<b>', '**'), '</b>', '**'),
    '<i>', '*'), '</i>', '*'),
    '<code>', '`'), '</code>', '`'),
    '&lt;', '<'), '&gt;', '>'), '&quot;', '"'), '&#39;', ''''), '&amp;', '&')
WHERE finish_reason = 'command'
  AND (
    content LIKE '%<b>%'
    OR content LIKE '%<i>%'
    OR content LIKE '%<code>%'
    OR content LIKE '%&lt;%'
    OR content LIKE '%&gt;%'
    OR content LIKE '%&quot;%'
    OR content LIKE '%&#39;%'
    OR content LIKE '%&amp;%'
  );
