// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Holds the count of hardcoded user-facing strings to a ceiling that only ever falls
// ABOUTME: A migration without one refills behind you and nothing says so until a user reads it

import path from 'path';
import { describe, it, expect } from 'vitest';
import { scanUntranslated, countsByScope } from '../untranslatedScan';

const ROOTS = [
  path.join(__dirname, '../../components'),
  path.join(__dirname, '../../onboarding'),
];

/**
 * The number of distinct hardcoded strings on ATHLETE-facing surfaces.
 *
 * Zero. The athlete-facing surface is fully translated, so this is no longer a
 * ratchet counting down — it is a floor. Any hardcoded string added to a
 * surface an athlete can reach fails this immediately, which is the whole
 * point of having driven it to zero rather than to "nearly none".
 *
 * It may only ever be lowered, and it cannot be. Raising it is the one change
 * this file exists to make someone argue for out loud.
 *
 * This is deliberately a count and not a list of permitted strings. An
 * allowlist would name the offenders and make them permanent furniture; a
 * ceiling says only "no more than this", so the pressure is always downward
 * and nothing gets blessed by being written down.
 *
 * The app defaults to French and answers coaching in French, so every string
 * left here is one a French athlete reads in English.
 *
 * Operator chrome is deliberately out of scope and ships in English: user
 * management, the eval harness, tool and harness config, claim verdicts.
 * Operators are internal and nobody needs Harness Config in Portuguese. That
 * is a product line, not a backlog — so it is excluded from the ceiling rather
 * than sitting in it forever as a number that can never reach zero. The count
 * is still reported below, so the decision stays visible instead of silent.
 */
const CEILING = 0;

describe('untranslated string ratchet', () => {
  it('carries no more hardcoded user-facing strings than the ceiling', () => {
    const hits = scanUntranslated(ROOTS);
    const { athlete: count, operator } = countsByScope(hits);
    // Printed on success too: a ratchet nobody can read the current value of
    // is just a test, and the number is the only honest progress report. The
    // operator figure rides along so the English-by-decision half stays
    // visible rather than quietly forgotten.
    console.log(
      `untranslated: athlete ${count} (ceiling ${CEILING}), operator ${operator} (English by decision)`,
    );

    if (count > CEILING) {
      // Name the files carrying the most, so the failure is a work list
      // rather than a number to bump.
      const byFile = new Map<string, number>();
      for (const hit of hits.filter((h) => h.scope === 'athlete')) {
        byFile.set(hit.file, (byFile.get(hit.file) ?? 0) + 1);
      }
      const worst = [...byFile.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 10)
        .map(([file, n]) => `  ${n}  ${path.relative(process.cwd(), file)}`)
        .join('\n');
      throw new Error(
        `${count} hardcoded athlete-facing strings, ceiling is ${CEILING}.\n` +
          `Translate them, or say why the ceiling should rise.\n${worst}`,
      );
    }

    expect(count).toBeLessThanOrEqual(CEILING);
  });

  it('still finds strings at all', () => {
    // A scanner that silently stopped matching would let the ceiling pass
    // vacuously and read as a finished migration. It reports zero only when
    // the job is genuinely done, and at that point this expectation is the
    // one that should be rewritten.
    const { athlete, operator } = countsByScope(scanUntranslated(ROOTS));
    expect(athlete + operator).toBeGreaterThan(0);
  });
});
