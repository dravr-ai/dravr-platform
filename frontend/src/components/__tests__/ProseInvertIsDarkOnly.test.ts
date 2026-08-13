// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Guards that prose-invert is never applied unconditionally
// ABOUTME: Unconditional prose-invert renders white table headers on the light canvas

import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const SRC = join(__dirname, '..', '..');

/**
 * Component sources only. Test files are skipped because this guard — and any
 * future test asserting on the class — necessarily writes the bare token in its
 * own body and would otherwise report itself.
 */
function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      return entry === '__tests__' ? [] : tsxFiles(full);
    }
    return /\.(tsx|ts)$/.test(entry) && !/\.test\.tsx?$/.test(entry) ? [full] : [];
  });
}

/**
 * `prose-invert` is Tailwind Typography's *dark* palette. Applied unconditionally
 * it wins in light mode too, painting `th` and headings white on the Boreal cream
 * canvas — the header row of a coach's table becomes invisible.
 *
 * Observed 2026-08-13: a five-column activity table rendered with an unreadable
 * header row in light mode. `darkMode: 'class'` is configured, so the variant
 * form `dark:prose-invert` is always the correct spelling.
 */
describe('prose-invert is dark-mode only', () => {
  it('never appears without the dark: variant', () => {
    const offenders: string[] = [];

    for (const file of tsxFiles(SRC)) {
      const text = readFileSync(file, 'utf8');
      text.split('\n').forEach((line, i) => {
        // Match `prose-invert` not immediately preceded by `dark:`.
        if (/(?<!dark:)\bprose-invert\b/.test(line)) {
          offenders.push(`${file.replace(SRC, 'src')}:${i + 1}`);
        }
      });
    }

    expect(offenders).toEqual([]);
  });

  it('still applies the dark palette in dark mode', () => {
    // The fix must not simply delete the class — dark mode still needs it.
    const messageItem = readFileSync(
      join(SRC, 'components', 'chat', 'MessageItem.tsx'),
      'utf8'
    );
    expect(messageItem).toContain('dark:prose-invert');
  });
});
