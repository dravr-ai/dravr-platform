-- ABOUTME: D11 — the opening sentence of a generated persona stops naming a role noun
-- ABOUTME: "You are a <type> coach specializing in X" becomes "You are a <type> specialist in X"
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- ADR-026 / D3: a prompt never tells the model what role to play with a noun.
-- `prompts/system/coach_generation.md` used to mandate «Start with "You are a
-- [specific type] coach specializing in [domain]..."», so every persona drafted
-- through `/agent create` (and `POST /api/coaches`) opens with that sentence.
-- The mandate is neutral now; the rows it already produced are not. This
-- rewrites the one clause, in place, and nothing else in the prompt.
--
-- Writing "you are an agent" instead would be worse than leaving it: the
-- identity anchor in `prompt_assembly.rs` suppresses a measured leak that a
-- role noun primes. The neutral form names the speciality, never the role.
--
-- SQLite has no `regexp` function in the build this server links, so the
-- rewrite is `instr`/`substr` here and `regexp_replace` in the PostgreSQL
-- twin. Both compute the same string: the text before the FIRST
-- ' coach specializing in ', then ' specialist in ', then the rest.
--
-- Only the opening sentence is eligible. The prefix must start with
-- 'You are a' (byte-exact) and carry no '.', no newline and no
-- ' specialist in ' — three guards that also make the statement idempotent:
-- once a row is rewritten its prefix holds ' specialist in ', so a second run
-- matches nothing. A row the mandate never produced matches nothing either.

UPDATE coaches
SET system_prompt =
        substr(system_prompt, 1, instr(system_prompt, ' coach specializing in ') - 1)
        || ' specialist in '
        || substr(
               system_prompt,
               instr(system_prompt, ' coach specializing in ')
                   + length(' coach specializing in ')
           )
WHERE instr(system_prompt, ' coach specializing in ') > 0
  AND substr(system_prompt, 1, 9) = 'You are a'
  AND instr(
          substr(system_prompt, 1, instr(system_prompt, ' coach specializing in ') - 1),
          '.'
      ) = 0
  AND instr(
          substr(system_prompt, 1, instr(system_prompt, ' coach specializing in ') - 1),
          char(10)
      ) = 0
  AND instr(
          substr(system_prompt, 1, instr(system_prompt, ' coach specializing in ') - 1),
          ' specialist in '
      ) = 0;
