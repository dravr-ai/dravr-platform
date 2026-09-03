// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Walks every athlete surface in French and fails on chrome still rendering its English value
// ABOUTME: The corpus is the oracle — a word list only ever catches the words someone thought of

import fs from 'fs';
import path from 'path';
import { test, expect, type Page } from '@playwright/test';
import { webRouteFor } from '@pierre/shared-constants';
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
  const keptAsIs = new Set<string>();
  for (const [key, english] of en) {
    const french = fr.get(key);
    if (french === undefined || english.includes('{{')) {
      continue;
    }
    // Only strings French actually renders differently can be caught this way;
    // a genuine cognate ("Admin") is indistinguishable from an untranslated one
    // and is skipped rather than guessed at.
    if (french === english) {
      keptAsIs.add(english);
    } else {
      flagged.add(english);
    }
  }
  // "Conversations" is "Discussions" under one key and French "Conversations"
  // under another; a page rendering the word may be showing either, so a value
  // the corpus keeps as-is anywhere cannot be an oracle.
  for (const value of keptAsIs) {
    flagged.delete(value);
  }
  return flagged;
}

/**
 * Every athlete surface, as the hash route that opens it.
 *
 * The connections pane is read from the surface registry rather than spelled
 * here: it moved from a top-level `#data-providers` tab to a section of
 * settings, and a stale literal would have swept a blank page — the shape of
 * a sweep that passes because nothing rendered.
 */
const SURFACES = [
  'chat',
  'discover',
  webRouteFor('data-providers') ?? 'settings',
  'groups',
  'notifications',
  'settings',
];

/**
 * Every text node under `root`, trimmed, that equals a string the corpus
 * translates into French. The chrome-only selector list above is right for
 * the dashboard's navigation; the onboarding wizard and the settings panes
 * put their copy in paragraphs, cards and step labels too, so those are read
 * whole — every text node, not a tag list somebody has to remember to extend.
 */
const TEXT_NODE_OFFENDERS = ({ values, root }: { values: string[]; root: string }): string[] => {
  const wanted = new Set(values);
  const base = document.querySelector(root) ?? document.body;
  const walker = document.createTreeWalker(base, NodeFilter.SHOW_TEXT);
  const hits: string[] = [];
  let node = walker.nextNode();
  while (node !== null) {
    const text = (node.textContent ?? '').replace(/\s+/g, ' ').trim();
    if (text && wanted.has(text)) {
      hits.push(text);
    }
    node = walker.nextNode();
  }
  return [...new Set(hits)];
};

/**
 * The onboarding wizard, one configuration per step.
 *
 * The wizard picks its step from `/api/me/onboarding-status`, so each step is
 * reached by answering that call, not by clicking through the previous ones.
 * This is the surface the 2026-09-01 flurry came from — stepper labels, the
 * profile cards, the PAR-Q questions and its Yes/No pair all rendered English
 * under French chrome while every gate said zero.
 */
const WIZARD_STEPS: {
  name: string;
  /** A corpus key that step alone renders — proof the step mounted before it is swept. */
  marker: string;
  steps: { step_id: string; status: string }[];
}[] = [
  { name: 'profile_type', marker: 'onboarding.imAnAthlete', steps: [] },
  {
    name: 'about_you',
    marker: 'onboarding.primarySportLabel',
    steps: [{ step_id: 'profile_type', status: 'complete' }],
  },
  {
    name: 'parq',
    marker: 'onboarding.parqIntro',
    steps: [
      { step_id: 'profile_type', status: 'complete' },
      { step_id: 'about_you', status: 'complete' },
    ],
  },
  {
    name: 'connect_provider',
    marker: 'onboarding.connectProviderIntro',
    steps: [
      { step_id: 'profile_type', status: 'complete' },
      { step_id: 'about_you', status: 'complete' },
      { step_id: 'parq', status: 'complete' },
    ],
  },
];

/** The French value under a dotted corpus key. */
function frenchValue(key: string): string {
  const value = key.split('.').reduce<unknown>(
    (node, part) => (node as Record<string, unknown> | undefined)?.[part],
    bundle('fr'),
  );
  if (typeof value !== 'string') {
    throw new Error(`corpus key ${key} is not a French string`);
  }
  return value;
}

/** The seven PAR-Q questions as the server serves them to a French athlete. */
function frenchParqQuestions(): { id: string; text: string }[] {
  const fr = bundle('fr') as { messaging: { intake: { parq: Record<string, string> } } };
  const questions = fr.messaging.intake.parq;
  return Object.entries(questions)
    .filter(([id]) => id !== 'intro')
    .map(([id, text]) => ({ id, text }));
}

/**
 * A sign-in that returns right after the click. The onboarding screens have
 * no `main` landmark, so `loginToDashboard` would wait on one forever; each
 * step asserts its own French heading instead. The mocked session does not
 * survive a navigation, so every step signs in afresh with its status stub
 * already registered — the route decides which step mounts.
 */
async function loginExpectingWizard(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForSelector('form', { timeout: 10_000 });
  await page.locator('input[name="email"]').fill('admin@test.com');
  await page.locator('input[name="password"]').fill('password123');
  await page.locator('form button[type="submit"]').first().click();
}

test.describe('French rendering sweep', () => {
  test('the sign-in page renders no English the corpus translates', async ({ page }) => {
    // The sweep logged in before it looked at anything, so the first screen an
    // athlete ever sees was the one surface it never read — and the divider
    // between the password form and the Google button said "or" under French
    // chrome for as long as the page has existed (carnet#206).
    const english = [...englishValuesWithFrenchTranslations()];
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem('pierre_app_language', 'fr');
      } catch { /* */ }
    });
    await page.goto('/');
    await expect(page.getByRole('button', { name: frenchValue('auth.signInButton') })).toBeVisible({
      timeout: 10_000,
    });
    const found = await page.evaluate(TEXT_NODE_OFFENDERS, { values: english, root: 'body' });
    expect(found, `Sign-in page rendering English:\n${found.join('\n')}`).toEqual([]);
  });

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

  test('every settings pane, Mémoire included, renders no English the corpus translates', async ({ page }) => {
    // The memory panel sat behind a tab, so the settings sweep above walked
    // past it while it read "Updated", "5 facts" and "North_star" under
    // French chrome. Each tab is opened and the pane read whole.
    const english = [...englishValuesWithFrenchTranslations()];
    await page.evaluate(() => { window.location.hash = '#settings'; });
    await page.waitForTimeout(1200);
    // The panes are buttons inside the settings navigation landmark, not
    // `role="tab"` elements.
    const tabs = page.getByRole('navigation', { name: /réglages/i }).getByRole('button');
    const count = await tabs.count();
    expect(count).toBeGreaterThan(3);
    const offenders: string[] = [];
    for (let i = 0; i < count; i += 1) {
      const tab = tabs.nth(i);
      const name = ((await tab.textContent()) ?? '').trim();
      await tab.click();
      await page.waitForTimeout(600);
      const found = await page.evaluate(TEXT_NODE_OFFENDERS, { values: english, root: 'main' });
      offenders.push(...found.map((t) => `settings/${name}: ${JSON.stringify(t)}`));
    }
    expect(offenders, `Settings panes rendering English:\n${offenders.join('\n')}`).toEqual([]);
  });

  test('the onboarding wizard renders no English the corpus translates, step by step', async ({ page }) => {
    const english = [...englishValuesWithFrenchTranslations()];
    await page.route('**/api/me/parq', async (route) => {
      if (route.request().method() !== 'GET') {
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ raised: 0 }) });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ questions: frenchParqQuestions() }),
      });
    });
    await page.route('**/api/me/about-you', async (route) => {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({}) });
    });
    const offenders: string[] = [];
    for (const step of WIZARD_STEPS) {
      await page.route('**/api/me/onboarding-status', async (route) => {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            needs_provider_connection: true,
            pillars_covered: 0,
            pillars_total: 7,
            onboarding_complete: false,
            steps: step.steps,
            chosen_channel: null,
          }),
        });
      });
      await loginExpectingWizard(page);
      // A sweep over a page the wizard never mounted on would pass for nothing.
      await expect(page.getByText(frenchValue(step.marker), { exact: false }).first()).toBeVisible({
        timeout: 10_000,
      });
      const found = await page.evaluate(TEXT_NODE_OFFENDERS, { values: english, root: 'body' });
      offenders.push(...found.map((t) => `onboarding/${step.name}: ${JSON.stringify(t)}`));
    }
    expect(offenders, `Onboarding rendering English:\n${offenders.join('\n')}`).toEqual([]);
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
