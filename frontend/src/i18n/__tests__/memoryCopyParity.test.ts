// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the memory screen's title, blurb and empty state to ONE catalogue key each, read by both clients
// ABOUTME: They existed twice — shell.* for the browser, app.* for the phone — and the copies had already drifted

import fs from 'fs';
import path from 'path';
import { describe, expect, it } from 'vitest';
import en from '../../../../packages/i18n/src/locales/en/translation.json';

const WEB_PANEL = path.join(__dirname, '../../components/memory/MemoryPanel.tsx');
// The settings pane table, which named the screen a third time in its own hint.
const PANE_TABLE = path.join(__dirname, '../../../../packages/shared-constants/src/surfaces.ts');
const MOBILE_SCREEN = path.join(
  __dirname,
  '../../../../frontend-mobile/src/screens/memory/MemoryScreen.tsx',
);

/**
 * The one key per string, and what each is for.
 *
 * The title read "Ce que TON coach retient de toi" in the browser and "Ce que
 * LE coach retient de toi" on the phone; the blurb differed by a whole rewrite.
 * Nothing kept them in step, so a wording change landed on one client only.
 */
const SHARED_KEYS = [
  'shell.memoryTitle',
  'app.memoryPanelBlurb',
  'shell.memoryEmpty',
  'shell.memoryEmptyHint',
  'shell.memoryEmptyFiltered',
  'shell.memoryEmptyFilteredHint',
  'shell.memoryShowAllKinds',
];

/** The second copies, retired: each said the same thing as a key above. */
const RETIRED_KEYS = [
  'app.whatCoachRemembers',
  'app.memoryBlurb',
  'app.noFactsYet',
  'app.memoryEmptyBlurb',
  // The third copy of the title: the settings pane's own hint said the same
  // sentence, byte for byte in fr/es/de/pt, and the pane now points at
  // `shell.memoryTitle` like both screens do.
  'settingsTabs.memoryHint',
];

function leaf(bundle: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    bundle,
  );
}

describe('memory screen copy parity', () => {
  const web = fs.readFileSync(WEB_PANEL, 'utf-8');
  const mobile = fs.readFileSync(MOBILE_SCREEN, 'utf-8');
  const surfaces = fs.readFileSync(PANE_TABLE, 'utf-8');

  it('has both clients read the same key for every string the screen shares', () => {
    for (const key of SHARED_KEYS) {
      expect(web, `web is missing ${key}`).toContain(`'${key}'`);
      expect(mobile, `mobile is missing ${key}`).toContain(`'${key}'`);
      expect(typeof leaf(en as Record<string, unknown>, key), key).toBe('string');
    }
  });

  it('leaves no second copy behind, in the catalogue or in either client', () => {
    for (const key of RETIRED_KEYS) {
      expect(leaf(en as Record<string, unknown>, key), key).toBeUndefined();
      expect(web, `web still reads ${key}`).not.toContain(key);
      expect(mobile, `mobile still reads ${key}`).not.toContain(key);
      expect(surfaces, `the pane table still reads ${key}`).not.toContain(key);
    }
  });
});
