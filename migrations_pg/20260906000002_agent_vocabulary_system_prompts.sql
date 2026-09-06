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
-- The SQLite twin does this with `instr`/`substr` because that build links no
-- `regexp` function; here `regexp_replace` expresses the same rewrite. The
-- non-greedy prefix stops at the FIRST ' coach specializing in ', which is the
-- match `strpos` finds in the WHERE clause, so the two agree row for row.
--
-- Only the opening sentence is eligible. The prefix must start with
-- 'You are a' (byte-exact) and carry no '.', no newline and no
-- ' specialist in ' — three guards that also make the statement idempotent:
-- once a row is rewritten its prefix holds ' specialist in ', so a second run
-- matches nothing. A row the mandate never produced matches nothing either.

UPDATE coaches
SET system_prompt = regexp_replace(
        system_prompt,
        E'^(You are a[^.\n]*?) coach specializing in ',
        E'\\1 specialist in '
    )
WHERE strpos(system_prompt, ' coach specializing in ') > 0
  AND left(system_prompt, 9) = 'You are a'
  AND strpos(
          left(system_prompt, strpos(system_prompt, ' coach specializing in ') - 1),
          '.'
      ) = 0
  AND strpos(
          left(system_prompt, strpos(system_prompt, ' coach specializing in ') - 1),
          E'\n'
      ) = 0
  AND strpos(
          left(system_prompt, strpos(system_prompt, ' coach specializing in ') - 1),
          ' specialist in '
      ) = 0;
