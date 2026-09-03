// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that no client links a 404 legal or help page, and that the privacy SETTING reads unlike the DOCUMENT
// ABOUTME: dravr.ai/help, /privacy and /terms all answered 404 on both apps, next to two rows sharing a name

import fs from 'fs';
import path from 'path';
import { describe, it, expect } from 'vitest';
import { SUPPORTED_LANGUAGES, defaultI18nConfig } from '@pierre/i18n';
import { HELP_URL, LEGAL_URL } from '@pierre/shared-constants';

/** Every athlete-facing source tree either client ships. */
const ROOTS = [
  path.join(__dirname, '../../../src'),
  path.join(__dirname, '../../../../frontend-mobile/src'),
  path.join(__dirname, '../../../../frontend-mobile/app'),
  path.join(__dirname, '../../../../packages/shared-constants/src'),
];

/** Addresses that answered 404 when the audit checked them, 2026-09-02. */
const DEAD_PATHS = ['dravr.ai/help', 'dravr.ai/privacy', 'dravr.ai/terms'];

function sourceFiles(root: string): string[] {
  if (!fs.existsSync(root)) return [];
  const stack = [root];
  const found: string[] = [];
  while (stack.length > 0) {
    const current = stack.pop() as string;
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== 'node_modules') stack.push(full);
      } else if (/\.(ts|tsx)$/.test(entry.name) && !/\.test\.|\.spec\./.test(entry.name)) {
        found.push(full);
      }
    }
  }
  return found;
}

function bundleFor(language: string): Record<string, Record<string, string>> {
  const resources = defaultI18nConfig.resources as Record<string, { translation: unknown }>;
  return resources[language].translation as Record<string, Record<string, string>>;
}

/** Lowercased words, punctuation dropped — "Privacy & Data" -> [privacy, data]. */
function words(label: string): string[] {
  return label
    .toLowerCase()
    .split(/[^\p{Letter}\p{Number}]+/u)
    .filter((word) => word.length > 0);
}

describe('help and legal destinations', () => {
  it('points both clients at a page that answers', () => {
    expect(HELP_URL).toBe('https://dravr.ai/docs');
    expect(LEGAL_URL).toBe('https://dravr.ai/docs');
  });

  it('leaves no client string resolving to a 404 page', () => {
    // Each client used to carry its own copy of the address, which is why the
    // same dead link shipped twice.
    const offenders: string[] = [];
    for (const root of ROOTS) {
      for (const file of sourceFiles(root)) {
        const source = fs.readFileSync(file, 'utf8');
        for (const dead of DEAD_PATHS) {
          if (source.includes(dead)) {
            offenders.push(`${path.relative(process.cwd(), file)} → ${dead}`);
          }
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});

describe('the privacy setting and the legal document read differently', () => {
  it.each([...SUPPORTED_LANGUAGES])(
    'differs by more than one word in %s',
    (language) => {
      // "Confidentialité et données" and "Conditions et confidentialité" sat in
      // adjacent sections, one word apart, and led to entirely different
      // places — one an in-app control, the other a page on the web.
      const bundle = bundleFor(language);
      const setting = bundle.settingsTabs.privacy;
      const document = bundle.about.legalDocuments;
      expect(setting).toBeTruthy();
      expect(document).toBeTruthy();

      const settingWords = new Set(words(setting));
      const documentWords = new Set(words(document));
      const shared = [...documentWords].filter((word) => settingWords.has(word));
      const distinct = [...documentWords].filter((word) => !settingWords.has(word));

      expect({ language, shared }).toEqual({ language, shared: [] });
      expect(distinct.length).toBeGreaterThan(1);
    },
  );
});
