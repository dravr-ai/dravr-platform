// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders every top-level surface in both themes for design review
// ABOUTME: Machine gates prove token mechanics; only a rendered screen proves it looks right

import { test, expect, type Page } from '@playwright/test';
import { webNavLabels } from '@pierre/shared-constants';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';
import { describeLayoutFailures, measurePageLayout } from './layout-gate';

// A colour or primitive change touches dozens of files at once. Type-checks and
// lint prove the classes exist; they cannot show that a status chip still reads
// as a status chip. This sweep exists so that review has pixels to look at, and
// so a reviewer can diff two runs instead of trusting a summary.
//
// Pixels for a reviewer were all it produced for a year, and its only assertion
// was that at least one surface loaded. It therefore screenshotted a Groups
// page welded to the viewport edge on every run and reported green. The walk
// now also measures each surface against the gutter its layout declares, so the
// one contract nobody restyles on purpose — content does not collide with the
// edge of its pane — fails the push instead of waiting for someone to open the
// artifact directory.
//
// The athlete-facing surfaces come from the shared registry, not from a list
// kept here. This file used to declare its own — a third surface declaration
// beside the registry and the sidebar — which is exactly the drift source the
// registry exists to remove: a surface added to the product reached neither
// this sweep nor anyone's attention.
const USER_SURFACES = webNavLabels();

// Admin surfaces are where the dense-data patterns live — tables, filter rows,
// status chips, verdict badges. They carry more colour per pixel than anything
// a regular user sees, so a palette change lands hardest here.
const ADMIN_SURFACES = [
  'Users',
  'Analytics',
  'Agent Store',
  'Tool Management',
  'Platform Settings',
  'Service Tokens',
  'Harness Config',
  'Agent Notes Audit',
] as const;

const THEMES = ['light', 'dark'] as const;

async function setTheme(page: Page, theme: (typeof THEMES)[number]) {
  // The app switches on a `.dark` class, not prefers-color-scheme, so
  // emulateMedia does nothing here — driving the class is the only honest way
  // to see the dark palette.
  await page.evaluate((t) => {
    document.documentElement.classList.toggle('dark', t === 'dark');
  }, theme);
  // Swapping the variables re-triggers every `transition-all duration-base`
  // on screen. Sampling before they land reads as a contrast bug that isn't
  // there — a 400ms wait had two CTAs showing interpolated mid-transition
  // colours that looked like a failed on-primary pairing.
  await page.waitForTimeout(1500);
}

async function sweep(
  page: Page,
  opts: { role: 'user' | 'admin'; surfaces: readonly string[]; theme: (typeof THEMES)[number] },
) {
  const { role, surfaces, theme } = opts;
  await setupDashboardMocks(page, { role });
  await loginToDashboard(page);

  // navigateToTab tries four selector strategies in turn. Against a tab that is
  // not present, each one burns the full default timeout, so a couple of
  // unreachable surfaces cost more than the whole sweep's budget. Everything
  // here runs against mocks on localhost; 5s is generous.
  page.setDefaultTimeout(5_000);

  const missed: string[] = [];
  const layoutFailures: string[] = [];
  for (const surface of surfaces) {
    try {
      await navigateToTab(page, surface);
    } catch {
      // A surface the mocks cannot reach is worth naming, not worth failing
      // on — a sweep that aborts halfway captures nothing for what follows.
      missed.push(surface);
      continue;
    }
    // Entrance animation is 500ms (DESIGN.md §7); capturing inside it yields a
    // half-faded screen that reads as a contrast bug.
    await page.waitForTimeout(700);
    await setTheme(page, theme);
    await page.screenshot({
      path: `design-sweep/${theme}/${role}-${surface.replace(/\s+/g, '-').toLowerCase()}.png`,
      fullPage: true,
    });
    // Measured after the same settle the capture uses, so the gate and the
    // screenshot describe the same frame.
    layoutFailures.push(...describeLayoutFailures(`${role}/${surface}`, await measurePageLayout(page)));
  }

  // Silence would let the sweep look complete while covering half the app.
  if (missed.length) {
    console.log(`design-sweep(${role}/${theme}): could not reach ${missed.join(', ')}`);
  }
  expect(missed.length).toBeLessThan(surfaces.length);
  // Reported together: a gate that stops at the first offender turns a
  // one-pass fix into one push per page.
  expect(layoutFailures, `layout contract broken on ${layoutFailures.length} surface(s):\n${layoutFailures.join('\n')}`).toEqual([]);
}

// Settings is not a nav label — it is reached from the gear — so the registry
// walk above never opens it, and the eight panes it holds were restyled twice
// (Boreal v2, then v2.1's sections) with no capture anyone could diff. This
// pass opens every row the settings menu renders, so a pane added to the
// shared pane declaration reaches the sweep without anyone listing it here.
async function sweepSettings(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'user' });
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const menu = page.getByTestId('settings-menu');
  await expect(menu).toBeVisible();
  const rows = menu.locator('[data-testid^="settings-menu-"]:not([id])');
  const ids = await rows.evaluateAll((els) =>
    els.map((el) => el.getAttribute('data-testid')?.replace('settings-menu-', '') ?? ''),
  );
  expect(ids.length).toBeGreaterThan(0);

  const layoutFailures: string[] = [];
  for (const id of ids) {
    await page.getByTestId(`settings-menu-${id}`).click();
    await page.waitForTimeout(700);
    await setTheme(page, theme);
    await page.screenshot({ path: `design-sweep/${theme}/user-settings-${id}.png`, fullPage: true });
    layoutFailures.push(...describeLayoutFailures(`user/settings-${id}`, await measurePageLayout(page)));
  }
  expect(layoutFailures, `layout contract broken on ${layoutFailures.length} pane(s):\n${layoutFailures.join('\n')}`).toEqual([]);
}

// The thread is the product screen, and the registry walk only ever sees the
// empty pane — the shared mocks hold no conversations. One conversation with
// a user turn, an agent turn with markdown, and a second exchange is enough
// to capture the reading column, the flat agent turn, the athlete bubble and
// the composer with text in it. Registered after the shared mocks so these
// routes win (Playwright matches the last registered route first).
const THREAD_CONVERSATIONS = {
  conversations: [
    {
      id: 'conv-sweep',
      title: 'Bloc seuil de septembre',
      coach_id: 'coach-camille',
      coach_name: 'Camille',
      created_at: '2026-09-01T10:00:00Z',
      updated_at: '2026-09-05T14:32:00Z',
      message_count: 4,
      unread_count: 0,
    },
  ],
  total: 1,
  limit: 50,
  offset: 0,
};
/**
 * The morning before the exchange below — enough rows to overflow the
 * transcript at the sweep's viewport. The layout gate then measures what only
 * a long thread shows: the pane must not scroll, because every athlete row
 * carries an absolutely positioned screen-reader label and the transcript's
 * scroller has to be the containing block that keeps them inside it.
 */
const THREAD_MORNING = Array.from({ length: 16 }, (_, i) => {
  const fromAthlete = i % 2 === 0;
  return {
    id: `sweep-morning-${i}`,
    conversation_id: 'conv-sweep',
    role: fromAthlete ? 'user' : 'assistant',
    content: fromAthlete
      ? `Question ${i / 2 + 1} : je garde la sortie longue de samedi ?`
      : 'Oui, garde-la telle quelle : la charge de la semaine tient et le repos de vendredi suffit.',
    created_at: new Date(Date.parse('2026-09-05T07:00:00Z') + i * 8 * 60_000).toISOString(),
    ...(fromAthlete ? {} : { model: 'claude-sonnet-5', execution_time_ms: 700 }),
  };
});

const THREAD_MESSAGES = {
  messages: [
    ...THREAD_MORNING,
    {
      id: 'sweep-1',
      conversation_id: 'conv-sweep',
      role: 'user',
      content: 'Je me sens un peu lourd ce matin, jambes fatiguées à l’échauffement.',
      created_at: '2026-09-05T09:20:00Z',
    },
    {
      id: 'sweep-2',
      conversation_id: 'conv-sweep',
      role: 'assistant',
      content:
        'C’est cohérent avec ta forme : TSB à −12 et HRV en léger recul cette nuit. Rien d’alarmant, mais je décale la séance seuil de demain à jeudi et je te mets une sortie souple demain.\n\n### Semaine ajustée\n1. Mardi — sortie souple, 1 h en Z2\n2. Jeudi — seuil, 3 × 12 min à 92 % de la FTP\n3. Samedi — sortie longue, 3 h',
      created_at: '2026-09-05T09:21:00Z',
      model: 'claude-sonnet-5',
      execution_time_ms: 1800,
    },
    {
      id: 'sweep-3',
      conversation_id: 'conv-sweep',
      role: 'user',
      content: 'Parfait, jeudi ça me va.',
      created_at: '2026-09-05T14:31:00Z',
    },
    {
      id: 'sweep-4',
      conversation_id: 'conv-sweep',
      role: 'assistant',
      content: 'Noté. Je te rappelle la séance mercredi soir avec la météo.',
      created_at: '2026-09-05T14:32:00Z',
      model: 'claude-sonnet-5',
      execution_time_ms: 900,
    },
  ],
};

async function sweepThread(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'user' });
  await page.route('**/api/chat/conversations/*/messages**', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(THREAD_MESSAGES) });
  });
  // The list pattern also matches the messages and read-marker URLs; those
  // fall through to the routes above and in the shared mocks.
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = request.url();
    if (url.includes('/messages') || url.includes('/read')) {
      await route.fallback();
      return;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(THREAD_CONVERSATIONS) });
  });
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await page.goto('/#chat/conv-sweep');
  await expect(page.getByTestId('message-row').first()).toBeVisible();
  await page.getByPlaceholder('Message Dravr...').first().fill('Et pour la nutrition jeudi ?');
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/user-chat-thread.png`, fullPage: true });
  const layoutFailures = describeLayoutFailures('user/chat-thread', await measurePageLayout(page));
  expect(layoutFailures, `layout contract broken:\n${layoutFailures.join('\n')}`).toEqual([]);
}

test.describe('design sweep', () => {
  // Each surface carries an entrance animation, a theme-transition settle and a
  // full-page capture. The default 30s cap is sized for a single interaction,
  // not a walk of the whole app.
  test.describe.configure({ timeout: 180_000 });

  for (const theme of THEMES) {
    test(`settings panes render in ${theme}`, async ({ page }) => {
      await sweepSettings(page, theme);
    });
    test(`the open thread renders in ${theme}`, async ({ page }) => {
      await sweepThread(page, theme);
    });
  }

  for (const theme of THEMES) {
    test(`user surfaces render in ${theme}`, async ({ page }) => {
      await sweep(page, { role: 'user', surfaces: USER_SURFACES, theme });
    });

    test(`admin surfaces render in ${theme}`, async ({ page }) => {
      await sweep(page, { role: 'admin', surfaces: ADMIN_SURFACES, theme });
    });
  }
});
