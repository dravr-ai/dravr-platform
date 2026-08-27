// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Walks every athlete surface in French and fails on chrome still rendering its English value
// ABOUTME: The corpus is the oracle — a word list only ever catches the words someone thought of

import fs from 'fs';
import path from 'path';
import { test, expect } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

// Read from disk rather than importing @pierre/i18n: Playwright's loader
// rejects the package's JSON imports without an import attribute, and the
// bundles are plain files anyway.
// ESM scope, so resolve from the Playwright rootDir rather than __dirname.
const LOCALES = path.resolve(process.cwd(), '../packages/i18n/src/locales');
const bundle = (lang: string): unknown =>
  JSON.parse(fs.readFileSync(path.join(LOCALES, lang, 'translation.json'), 'utf-8'));

/**
 * Why this reads rendered output, and why the corpus is the oracle.
 *
 * The untranslated-string ratchet scans source, and it reported athlete zero
 * three separate times while the login page still read "Sign in" under
 * "Se connecter". Every gap was a shape the regex could not see: lowercase text
 * after a `<br />`, literals inside ternaries, double-quoted ternaries, copy
 * running into `{expr}`, prose in template literals. Each was found by opening
 * the page, never by the gate.
 *
 * The first version of this test looked for English marker words. It passed
 * with "Data Providers" sitting in the French sidebar, because nobody had put
 * "data" or "providers" in the list — the same vacuous-guard failure the
 * ratchet had, wearing different clothes.
 *
 * So the question it asks now is exact: for every string the corpus translates,
 * does the page render the English one? That needs no vocabulary and cannot go
 * stale, because the corpus grows with the product.
 */
function englishValuesWithFrenchTranslations(): Set<string> {
  const flatten = (node: unknown, out: Map<string, string>, prefix = ''): Map<string, string> => {
    if (typeof node === 'string') {
      out.set(prefix, node);
      return out;
    }
    for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
      flatten(value, out, prefix === '' ? key : `${prefix}.${key}`);
    }
    return out;
  };
  const en = flatten(bundle('en'), new Map());
  const fr = flatten(bundle('fr'), new Map());

  const flagged = new Set<string>();
  for (const [key, english] of en) {
    const french = fr.get(key);
    // Only strings French actually renders differently can be caught this way;
    // a genuine cognate ("Admin") is indistinguishable from an untranslated one
    // and is skipped rather than guessed at.
    if (french !== undefined && french !== english && !english.includes('{{')) {
      flagged.add(english);
    }
  }
  return flagged;
}

const SURFACES = ['chat', 'discover', 'data-providers', 'groups', 'notifications', 'settings'];

test.describe('French rendering sweep', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem('pierre_app_language', 'fr');
      } catch { /* */ }
    });
    await setupDashboardMocks(page, { role: 'user' });
    await loginToDashboard(page);
  });

  test('no athlete surface renders a string the corpus translates', async ({ page }) => {
    const english = [...englishValuesWithFrenchTranslations()];
    expect(english.length).toBeGreaterThan(500);

    const offenders: string[] = [];
    for (const surface of SURFACES) {
      await page.evaluate((hash) => { window.location.hash = `#${hash}`; }, surface);
      await page.waitForTimeout(1200);

      // Chrome only. Coach names and descriptions come from the database in
      // English and are not the corpus's to translate.
      const found = await page.evaluate((values: string[]) => {
        const wanted = new Set(values);
        const nodes = document.querySelectorAll(
          'aside nav button, main button, main h1, main h2, main h3, main h4, main label, main [role="tab"], main th',
        );
        const hits: string[] = [];
        for (const node of Array.from(nodes)) {
          const text = (node.textContent ?? '').replace(/\s+/g, ' ').trim();
          if (text && wanted.has(text)) {
            hits.push(text);
          }
        }
        return [...new Set(hits)];
      }, english);

      offenders.push(...found.map((t) => `${surface}: ${JSON.stringify(t)}`));
    }

    expect(
      offenders,
      `Chrome rendering English where French exists:\n${offenders.join('\n')}`,
    ).toEqual([]);
  });

  test('no surface paints a raw corpus key', async ({ page }) => {
    // A key reaching the screen is worse than an untranslated string: the user
    // reads "shell.billingUpgradeProfessional" on a checkout button. It happened
    // — a bulk edit meant for a module-level lookup table ran over whole files
    // and flattened 52 live `t('x')` calls into the bare string `'x'`. The e2e
    // suite caught 9 of them, because only 9 had an assertion aimed at that
    // exact text; the other 43 would have shipped.
    //
    // Nothing about this one is locale-specific: a key is wrong in every
    // language, so it is asserted over the whole rendered document.
    const KEY_SHAPE = /^[a-z][A-Za-z]*\.[a-z][A-Za-z0-9]*$/;
    const leaked: string[] = [];

    for (const surface of SURFACES) {
      await page.evaluate((hash) => { window.location.hash = `#${hash}`; }, surface);
      await page.waitForTimeout(1000);

      const found = await page.evaluate((shape: string) => {
        const rx = new RegExp(shape);
        const hits: string[] = [];
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        let node = walker.nextNode();
        while (node !== null) {
          const text = (node.textContent ?? '').trim();
          if (text && rx.test(text)) {
            hits.push(text);
          }
          node = walker.nextNode();
        }
        return [...new Set(hits)];
      }, KEY_SHAPE.source);

      leaked.push(...found.map((t) => `${surface}: ${t}`));
    }

    expect(leaked, `Corpus keys painted as text:\n${leaked.join('\n')}`).toEqual([]);
  });
});
