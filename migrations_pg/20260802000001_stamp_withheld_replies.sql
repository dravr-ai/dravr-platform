-- ABOUTME: Stamps historically withheld assistant rows so the replay path can drop them
-- ABOUTME: Backfill for WITHHELD_REPLY_FINISH_REASON; forward writes are stamped by the pipeline

-- A reply withheld at the response boundary persists a localized apology in
-- place of the model output. The athlete saw it, so the row stays — but it is
-- first-person narration about the platform failing, and replaying it into
-- later prompts teaches the coach that its own output gets blocked (the same
-- self-referential-failure class as the 2026-07-23 learned-helplessness
-- incident). The pipeline now stamps these rows at write time; this backfills
-- the ones already in the table, which are re-injected into every subsequent
-- prompt of their conversation until stamped.
--
-- Matched on the LEADING CLAUSE of each locale default rather than the whole
-- string: the tail ("Renvoie ton dernier message…") is the part most likely to
-- be reworded in dravr-contremaitre. None of the prefixes contains a LIKE
-- wildcard (% or _), so no escaping is required.
UPDATE chat_messages
SET finish_reason = 'reply_withheld'
WHERE role = 'assistant'
  AND finish_reason IS DISTINCT FROM 'reply_withheld'
  AND (
    content LIKE 'Ma réponse n''est pas passée%'
    OR content LIKE 'My reply didn''t go through%'
    OR content LIKE 'Mi respuesta no salió%'
    OR content LIKE 'Meine Antwort ging nicht raus%'
    OR content LIKE 'A minha resposta não seguiu%'
  );
