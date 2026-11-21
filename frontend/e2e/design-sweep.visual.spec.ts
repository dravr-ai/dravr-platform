// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders every top-level surface in both themes for design review
// ABOUTME: Machine gates prove token mechanics; only a rendered screen proves it looks right

import { test, expect, type Page } from '@playwright/test';
import { webNavLabels } from '@pierre/shared-constants';
import type { UserRole } from '@pierre/shared-types';
import {
  setupDashboardMocks,
  loginToDashboard,
  signInThroughHostedPage,
  mockHomeFirstArrival,
  mockHomeReturning,
  navigateToTab,
  ONBOARDED_STATUS,
  TOKENS_TAB_LABEL,
  waitForLoginScreen,
} from './test-helpers';
import { setupBillingMocks } from './billing-mocks';
import { mockSetupStatus, mockSignedOut, mockSignupConfig, takeoverToken } from './auth-mocks';
import { describeLayoutFailures, measurePageLayout } from './layout-gate';
import { mockCoach } from './coach-home-mocks';
import { PENDING_REQUEST, SEAT_CONVERSATION_ID, setupCoachSeatMocks } from './coach-seat-mocks';

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
  TOKENS_TAB_LABEL,
  'Harness Config',
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
  opts: { role: UserRole; surfaces: readonly string[]; theme: (typeof THEMES)[number] },
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
  // The messaging pane's chat-app groups hold one linked app and one still to
  // link, so both row shapes reach the capture. The onboarding chat-app steps
  // read the same channel list, so the record says they were skipped — else
  // they would intercept the sign-in.
  await page.route('**/api/me/onboarding-status', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        needs_provider_connection: false,
        steps: [
          ...ONBOARDED_STATUS.steps,
          { step_id: 'messaging_channel', status: 'skipped' },
          { step_id: 'messaging_configure', status: 'skipped' },
        ],
      }),
    }),
  );
  await page.route('**/api/messaging/channels/available', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        { channel: 'telegram', display_name: 'Telegram', method: 'deep_link', recommended: true },
        { channel: 'slack', display_name: 'Slack', method: 'oauth', recommended: false },
      ]),
    }),
  );
  await page.route('**/api/messaging/links', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        links: [{ channel: 'telegram', channel_user_id: 'tg-42', display_name: '@athlete', linked_at: '2026-09-01T10:00:00Z' }],
      }),
    }),
  );
  // The privacy pane's health records block holds one result waiting for
  // review (with a check it failed), one lab report and one served result, so
  // each of its row shapes reaches the capture rather than its empty lines.
  const flat = {
    code_system: 'http://loinc.org', code: '718-7', display: 'Hemoglobin', value: 150, unit: 'g/L',
    value_code: null, interpretation: null, ref_low: 135, ref_high: 175, status: 'final', components: [],
    effective_at: '2026-05-05',
  };
  const pending = {
    id: 'rec-1', flat, checks: ['flag_not_printed'], locator: 'text:page=1;line=4',
    review_decision: null, effective_at: '2026-05-05T00:00:00Z',
  };
  const labDocument = {
    id: 'doc-1', status: 'read', pages: 2, size_bytes: 48_000, engine: 'rules', flagged_lines: 0,
    skipped: 0, dropped: 0, model_failure: null, received_at: '2026-11-20T15:00:00Z',
  };
  const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  await page.route('**/api/me/health-records/review', (route) => route.fulfill(json({ pending: [pending] })));
  await page.route('**/api/me/health-records?*', (route) =>
    route.fulfill(
      json({
        records: [
          {
            id: 'served-1', resource_type: 'Observation', tier: 't2', effective_at: '2026-05-05T00:00:00Z',
            source: 'upload', resource: {}, flat, derivation: 'confirmed', read_by: 'rules',
          },
        ],
      }),
    ),
  );
  await page.route('**/api/me/lab-documents', (route) =>
    route.fulfill(json({ documents: [{ document: labDocument, records: [pending], new: false }] })),
  );
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
    if (id === 'privacy') {
      // The health records block on its own, then the confirmation its
      // delete asks for: both are read in this pane only.
      const records = page.getByTestId('health-records-settings');
      await expect(page.getByTestId('health-review-rec-1')).toBeVisible();
      await records.scrollIntoViewIfNeeded();
      await records.screenshot({ path: `design-sweep/${theme}/user-settings-health-records.png` });
      await page.getByTestId('health-document-delete-doc-1').click();
      await expect(page.getByRole('dialog')).toBeVisible();
      await page.waitForTimeout(400);
      await page.screenshot({ path: `design-sweep/${theme}/user-settings-health-records-delete.png` });
      await page.keyboard.press('Escape');
      await expect(page.getByRole('dialog')).toHaveCount(0);
    }
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
      agent_id: 'coach-camille',
      agent_name: 'Camille',
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

// Onboarding's Your team step (carnet#1095) is not a nav surface, so the
// registry walk never reaches it: an athlete meets it once, right after the
// connect step. This pass opens it the way that athlete does — every earlier
// step done, no team step recorded — on a proposal where every rule fired
// (two members ticked, Mobility offered unticked), then the Browse all agents
// sheet over it. Onboarding has no page shell for the gutter gate to read, so
// the one layout contract checked is that nothing scrolls the page sideways.
const TEAM_LEAD = {
  agent: { id: 'agent-endurance', title: 'Endurance', category: 'training' },
  match_score: 0.92,
  reason: 'You ride four times a week and named a long event in June. It reads every session and plans toward that day.',
};
const TEAM_PROPOSAL = {
  profile: {
    has_profile: true,
    window_days: 90,
    total_activities: 46,
    primary_sport: 'ride',
    sport_mix: [
      { sport: 'ride', count: 33, share: 0.72 },
      { sport: 'run', count: 10, share: 0.21 },
      { sport: 'strength_training', count: 3, share: 0.07 },
    ],
  },
  goal: 'a gran fondo in June',
  agents: [TEAM_LEAD],
  team: {
    lead: TEAM_LEAD,
    members: [
      {
        agent: { id: 'agent-fuelling', title: 'Fuelling', category: 'nutrition' },
        rule: 'goal',
        preselected: true,
        reason: 'A five-hour day is won or lost on what you eat in the last two hours.',
      },
      {
        agent: { id: 'agent-recovery', title: 'Recovery', category: 'recovery' },
        rule: 'sleep_source',
        source: 'Garmin',
        preselected: true,
        reason: 'Garmin sends your sleep and HRV, so it has something real to read every morning.',
      },
      {
        agent: { id: 'agent-mobility', title: 'Mobility', category: 'mobility' },
        rule: 'strength',
        preselected: false,
        reason: 'Short routines that pair with your strength days. Add it now or later.',
      },
    ],
  },
};
const TEAM_STORE_LISTINGS = ['Endurance', 'Fuelling', 'Mind'].map((title, i) => ({
  id: `listing-${i}`,
  title,
  description: `${title} agent from the catalogue.`,
  category: ['training', 'nutrition', 'custom'][i],
  tags: [],
  sample_prompts: [],
  token_count: 900,
  install_count: 12 + i,
  icon_url: null,
  published_at: '2026-09-01T00:00:00Z',
  author_id: null,
}));

async function sweepOnboardingTeam(page: Page, theme: (typeof THEMES)[number]) {
  await page.addInitScript(() => {
    for (const step of ['profile_type_chosen', 'about_you_done', 'parq_done', 'check_in_done']) {
      window.localStorage.setItem(`dravr.${step}.user-123`, '1');
    }
  });
  await setupDashboardMocks(page, { role: 'user', displayName: 'Camille' });
  // Registered after the shared mocks, so these win.
  await page.route('**/api/me/onboarding-status', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_provider_connection: false, steps: [] }),
    }),
  );
  await page.route('**/api/agents/proposal', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(TEAM_PROPOSAL) }),
  );
  await page.route('**/api/store/agents**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ agents: TEAM_STORE_LISTINGS, next_cursor: null, has_more: false }),
    }),
  );
  await page.goto('/');
  await signInThroughHostedPage(page);
  page.setDefaultTimeout(5_000);
  await expect(page.getByRole('heading', { name: "Camille, here's your starting team" })).toBeVisible({
    timeout: 15_000,
  });

  const sidewaysScroll = () =>
    page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/user-onboarding-team.png`, fullPage: true });
  expect(await sidewaysScroll(), 'the Your team step scrolls sideways').toBeLessThanOrEqual(0);

  await page.getByRole('button', { name: 'Browse all agents' }).click();
  await expect(page.getByRole('dialog', { name: 'All agents' })).toBeVisible();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `design-sweep/${theme}/user-onboarding-team-browse.png`, fullPage: true });
  expect(await sidewaysScroll(), 'the Browse all agents sheet scrolls sideways').toBeLessThanOrEqual(0);
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
    test(`the onboarding Your team step renders in ${theme}`, async ({ page }) => {
      await sweepOnboardingTeam(page, theme);
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

/** Whether the page scrolls sideways, in px past the viewport. */
async function sidewaysOverflow(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
}

// Home's first arrival (carnet#1098, carnet#1528 P5·A): the thread opens on the
// lead's welcome — the figures it read of the synced history, the team and
// its starters as links — and Getting started leads Today, the
// state no registry walk reaches, since the shared mocks model an established
// athlete. Wide (1440), Today is the docked panel; narrow (390), the thread
// leads with the Today line carrying the count, and its sheet opens on Getting
// started. `provider: false` is the same arrival with nothing connected: the
// plain greeting and « Connect my watch ». Both widths pass the page's layout
// contract.
async function sweepHomeFirstArrival(page: Page, theme: (typeof THEMES)[number], provider: boolean) {
  const name = provider ? 'user-home-first-arrival' : 'user-home-first-arrival-no-provider';
  await page.setViewportSize({ width: 1440, height: 900 });
  await setupDashboardMocks(page, { role: 'user', displayName: 'Camille' });
  await mockHomeFirstArrival(page, { provider });
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await expect(page.getByTestId('home-getting-started')).toBeVisible();
  await expect(page.getByTestId('home-team')).toBeVisible();
  await expect(page.getByTestId('welcome-starters')).toBeVisible();
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/${name}.png`, fullPage: true });
  const wide = describeLayoutFailures(`user/${name}`, await measurePageLayout(page));
  expect(wide, `layout contract broken:\n${wide.join('\n')}`).toEqual([]);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByTestId('home-today-peek-progress')).toHaveText(provider ? '1 of 5' : '0 of 5');
  await expect(page.getByTestId('welcome-starters')).toBeVisible();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `design-sweep/${theme}/${name}-phone.png`, fullPage: true });
  expect(await sidewaysOverflow(page), 'the thread scrolls sideways at 390px').toBeLessThanOrEqual(0);
  await page.getByTestId('home-today-peek').click();
  await expect(page.getByRole('dialog', { name: 'Today' }).getByTestId('home-getting-started')).toBeVisible();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `design-sweep/${theme}/${name}-phone-today.png`, fullPage: true });
  expect(await sidewaysOverflow(page), 'the Today sheet scrolls sideways at 390px').toBeLessThanOrEqual(0);
}

// A returning athlete's Home (carnet#1528 P5·C): they answered the welcome, so
// Today leads with today's session and the team folds to one row, and Getting
// started is the slim meter under the conversation's header.
async function sweepHomeReturning(page: Page, theme: (typeof THEMES)[number]) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await setupDashboardMocks(page, { role: 'user', displayName: 'Camille' });
  await mockHomeReturning(page);
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await expect(page.getByTestId('home-getting-started-meter')).toBeVisible();
  await expect(page.getByTestId('home-team')).toBeVisible();
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/user-home-returning.png`, fullPage: true });
  const wide = describeLayoutFailures('user/home-returning', await measurePageLayout(page));
  expect(wide, `layout contract broken:\n${wide.join('\n')}`).toEqual([]);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByTestId('home-getting-started-meter')).toBeVisible();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `design-sweep/${theme}/user-home-returning-phone.png`, fullPage: true });
  expect(await sidewaysOverflow(page), 'the thread scrolls sideways at 390px').toBeLessThanOrEqual(0);
  await page.getByTestId('home-today-peek').click();
  await expect(page.getByRole('dialog', { name: 'Today' }).getByTestId('home-today')).toBeVisible();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `design-sweep/${theme}/user-home-returning-phone-today.png`, fullPage: true });
  expect(await sidewaysOverflow(page), 'the Today sheet scrolls sideways at 390px').toBeLessThanOrEqual(0);
}

test.describe('design sweep — Home first arrival', () => {
  test.describe.configure({ timeout: 180_000 });
  for (const theme of THEMES) {
    test(`Home's first arrival renders in ${theme}`, async ({ page }) => {
      await sweepHomeFirstArrival(page, theme, true);
    });
    test(`Home's first arrival with no provider renders in ${theme}`, async ({ page }) => {
      await sweepHomeFirstArrival(page, theme, false);
    });
    test(`a returning athlete's Home renders in ${theme}`, async ({ page }) => {
      await sweepHomeReturning(page, theme);
    });
  }
});

// A coach's Home (carnet#1250, #1252, #1253): À traiter with an athlete open,
// the team table and the season, each on a roster tall enough that the list,
// the table and the pane have to scroll inside themselves — the contract a
// bleed surface owns — then the same three on a phone. The shared mocks model
// an athlete, so none of this is reached by the registry walk.
async function captureCoachPane(page: Page, theme: (typeof THEMES)[number], name: string) {
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/coach-home-${name}.png`, fullPage: true });
  const failures = describeLayoutFailures(`coach-home/${name}`, await measurePageLayout(page));
  expect(failures, `layout contract broken:\n${failures.join('\n')}`).toEqual([]);
}

async function sweepCoachHome(page: Page, theme: (typeof THEMES)[number]) {
  await mockCoach(page, { fillers: 30 });
  page.setDefaultTimeout(5_000);
  await expect(page.getByTestId('coach-queue')).toBeVisible();

  await page.getByTestId('coach-queue-row-u-lea').click();
  await expect(page.getByTestId('form-strip')).toBeVisible();
  await captureCoachPane(page, theme, 'queue');

  await page.getByTestId('coach-see-all').click();
  await expect(page.getByTestId('coach-roster')).toBeVisible();
  await captureCoachPane(page, theme, 'team');

  await page.getByTestId('coach-roster-back').click();
  await page.getByTestId('coach-view-season').click();
  await expect(page.getByTestId('coach-season-ahead')).toBeVisible();
  await captureCoachPane(page, theme, 'season');
  await page.getByTestId('coach-season-scale-12w').click();
  await captureCoachPane(page, theme, 'season-12w');

  await page.getByTestId('coach-home-switch-me').click();
  await expect(page.getByTestId('home-page')).toBeVisible();
  await captureCoachPane(page, theme, 'me');

  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByTestId('coach-home-switch-athletes').click();
  await expect(page.getByTestId('coach-queue')).toBeVisible();
  await captureCoachPane(page, theme, 'queue-phone');
  await page.getByTestId('coach-queue-row-u-julien').click();
  await expect(page.getByTestId('coach-athlete-pane')).toBeVisible();
  await captureCoachPane(page, theme, 'athlete-phone');
  await page.getByTestId('coach-pane-back').click();
  await page.getByTestId('coach-see-all').click();
  await expect(page.getByTestId('coach-roster-list')).toBeVisible();
  await captureCoachPane(page, theme, 'team-phone');
  await page.getByTestId('coach-view-season').click();
  await expect(page.getByTestId('coach-season-ahead')).toBeVisible();
  await captureCoachPane(page, theme, 'season-phone');
}

test.describe('design sweep — Coach Home', () => {
  test.describe.configure({ timeout: 180_000 });
  for (const theme of THEMES) {
    test(`a coach's Home renders in ${theme}`, async ({ page }) => {
      await sweepCoachHome(page, theme);
    });
  }
});

// The pending coach Home (carnet#1529): a coach without coach access who does
// not train, before the request, after it and after the grant, then the same
// on a phone; and their group room once coached, where no provider nudge
// shows. The shared mocks model an athlete, so none of this is reached by the
// registry walk.
async function capturePendingCoach(page: Page, theme: (typeof THEMES)[number], name: string) {
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/pending-coach-${name}.png`, fullPage: true });
  const failures = describeLayoutFailures(`pending-coach/${name}`, await measurePageLayout(page));
  expect(failures, `layout contract broken:\n${failures.join('\n')}`).toEqual([]);
}

async function sweepPendingCoachHome(page: Page, theme: (typeof THEMES)[number], width: number) {
  await page.setViewportSize({ width, height: width < 768 ? 844 : 900 });
  const suffix = width < 768 ? '-phone' : '';
  const record = await setupCoachSeatMocks(page, { trains: false });
  await loginToDashboard(page);
  page.setDefaultTimeout(10_000);
  await expect(page.getByTestId('coach-setup')).toBeVisible();
  await expect(page.getByTestId('coach-setup-row-seat').getByTestId('coach-access-request')).toBeVisible();
  await capturePendingCoach(page, theme, `before-request${suffix}`);

  await page.getByTestId('coach-setup-row-seat').getByTestId('coach-access-request').click();
  await expect(page.getByTestId('coach-seat-row')).toHaveAttribute('data-state', 'pending');
  await capturePendingCoach(page, theme, `after-request${suffix}`);

  await record.grant();
  await page.evaluate(() => {
    window.dispatchEvent(new Event('visibilitychange'));
    window.dispatchEvent(new Event('focus'));
  });
  await expect(page.getByTestId('coach-home')).toBeVisible({ timeout: 15_000 });
  await capturePendingCoach(page, theme, `after-grant${suffix}`);
}

async function sweepUntrainedCoachRoom(page: Page, theme: (typeof THEMES)[number], width: number) {
  await page.setViewportSize({ width, height: width < 768 ? 844 : 900 });
  const suffix = width < 768 ? '-phone' : '';
  await setupCoachSeatMocks(page, { request: PENDING_REQUEST, trains: false });
  await loginToDashboard(page);
  await page.evaluate((id) => {
    window.location.hash = `#chat/${id}`;
  }, SEAT_CONVERSATION_ID);
  await expect(page.getByTestId('thread-header')).toBeVisible();
  await expect(page.getByTestId('connect-provider-banner')).toHaveCount(0);
  await capturePendingCoach(page, theme, `room${suffix}`);
}

test.describe('design sweep — pending coach Home', () => {
  test.describe.configure({ timeout: 180_000 });
  for (const theme of THEMES) {
    for (const width of [1440, 390]) {
      test(`the pending coach Home renders in ${theme} at ${width}`, async ({ page }) => {
        await sweepPendingCoachHome(page, theme, width);
      });
      test(`a non-training coach's room renders in ${theme} at ${width}`, async ({ page }) => {
        await sweepUntrainedCoachRoom(page, theme, width);
      });
    }
  }
});

// ---------------------------------------------------------------------------
// Tone surfaces (carnet#1584). Every banner, notice, inline error, medallion,
// toast and state button reads ALERT_TONE, BADGE_TONE or a Button variant,
// and most of them only appear in a state the registry walk never reaches: a
// failed form, a saved setting, a payment problem, a provider's sign-in. Each
// pass below drives one into view and captures it, so a reviewer sees the
// tones on the surfaces they live on, in both schemes.
// ---------------------------------------------------------------------------

async function captureTone(page: Page, theme: (typeof THEMES)[number], name: string) {
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/tones-${name}.png` });
}

const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

/** The sign-up form's own error alert, after a mismatched confirmation. */
async function sweepRegisterError(page: Page, theme: (typeof THEMES)[number]) {
  await mockSetupStatus(page);
  await mockSignupConfig(page);
  await page.goto('/');
  await waitForLoginScreen(page, 10_000);
  await page.getByText("Don't have an account?").click();
  await page.locator('input[name="email"]').fill('newuser@test.com');
  await page.locator('input[name="password"]').fill('SecurePassword123');
  await page.locator('input[name="confirmPassword"]').fill('DifferentPassword');
  await page.getByRole('button', { name: 'Create Account' }).click();
  await expect(page.getByText('Passwords do not match')).toBeVisible();
  await captureTone(page, theme, 'auth-register-error');
}

/** Platform Settings: the auto-approval switch on, its success status and the saved alert. */
async function sweepPlatformSettings(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'admin' });
  let enabled = false;
  await page.route('**/api/admin/settings/auto-approval', async (route, request) => {
    if (request.method() === 'PUT') enabled = Boolean(request.postDataJSON()?.enabled);
    await route.fulfill(
      json({ data: { enabled, auto_approve_domains: [], overridden_by_env: false, description: 'Auto-approve new users' } }),
    );
  });
  await page.route('**/api/admin/settings/signup', (route) =>
    route.fulfill(
      json({
        data: {
          open: true,
          daily_limit: 50,
          activate_on_verified_email: true,
          counted: 3,
          counted_since: '2026-11-15T00:00:00Z',
          turnstile_configured: false,
          description: 'Sign-up guards',
        },
      }),
    ),
  );
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await navigateToTab(page, 'Platform Settings');
  await page.getByRole('switch', { name: 'Auto-Approve Registrations' }).click();
  const saved = page.getByText('Setting updated successfully.');
  await expect(saved).toBeVisible();
  await expect(page.getByText(/Auto-approval is enabled/)).toBeVisible();
  await saved.scrollIntoViewIfNeeded();
  await captureTone(page, theme, 'admin-platform-settings');
}

/** Tool Management with a globally disabled tool: the error banner and its chips. */
async function sweepToolBanner(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'admin' });
  await page.route('**/api/admin/tools/global-disabled', (route) =>
    route.fulfill(json({ success: true, data: { disabled_tools: ['calculate_metrics'], count: 1 } })),
  );
  await page.route('**/api/admin/tools/tenant/*/summary', (route) =>
    route.fulfill(
      json({
        success: true,
        data: {
          tenant_id: 'user-123',
          total_tools: 2,
          enabled_tools: 1,
          disabled_tools: 1,
          overridden_tools: 0,
          globally_disabled_count: 1,
          plan_restricted_count: 0,
        },
      }),
    ),
  );
  await page.route(/\/api\/admin\/tools\/tenant\/[^/]+$/, (route) =>
    route.fulfill(
      json({
        success: true,
        data: [
          { tool_name: 'get_activities', display_name: 'Get Activities', description: 'Retrieve fitness activities', category: 'Activity', is_enabled: true, source: 'Default', min_plan: 'Free' },
          { tool_name: 'calculate_metrics', display_name: 'Calculate Metrics', description: 'Calculate TRIMP and TSS', category: 'Analytics', is_enabled: false, source: 'GlobalDisabled', min_plan: 'Enterprise' },
        ],
      }),
    ),
  );
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await navigateToTab(page, 'Tool Management');
  await expect(page.getByText('Globally Disabled Tools')).toBeVisible();
  await captureTone(page, theme, 'admin-tool-banner');
  // The rows' switches (ui/Switch): one on, one disabled off.
  await page.getByRole('switch', { name: 'Toggle Calculate Metrics' }).scrollIntoViewIfNeeded();
  await captureTone(page, theme, 'admin-tool-switches');
}

/** Pending users: the success approve button and the quiet danger-outline suspend, at rest and hovered. */
async function sweepPendingUsers(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'admin' });
  await page.route('**/api/admin/pending-users', (route) =>
    route.fulfill(
      json({
        success: true,
        message: 'Retrieved 1 pending users',
        data: {
          count: 1,
          users: [
            { id: 'user-1', email: 'pending1@example.com', display_name: 'Pending User 1', status: 'pending', tier: 'trial', created_at: '2026-11-14T10:00:00Z', last_active_at: null },
          ],
        },
      }),
    ),
  );
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await navigateToTab(page, 'Users');
  await expect(page.getByText('pending1@example.com')).toBeVisible();
  await captureTone(page, theme, 'admin-users-pending');
  await page.getByRole('button', { name: 'Suspend' }).first().hover();
  await page.waitForTimeout(400);
  await captureTone(page, theme, 'admin-users-pending-suspend-hover');
}

/** Billing in dunning: the payment-problem error banner, its danger button, the plan cards. */
async function sweepBillingProblem(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'user', email: 'pro@pierre.dev' });
  await setupBillingMocks(page, { noSubscription: false, subscriptionStatus: 'past_due' });
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await page.locator('button', { hasText: 'Usage' }).first().click();
  await expect(page.getByRole('heading', { name: /Payment problem/i })).toBeVisible();
  await captureTone(page, theme, 'user-billing-payment-problem');
}

/** The login takeover's page: its connected and closed medallions. */
async function sweepSignInCheck(page: Page, theme: (typeof THEMES)[number], outcome: 'connected' | 'closed') {
  const token = takeoverToken();
  await mockSetupStatus(page);
  await mockSignedOut(page);
  await page.routeWebSocket('**/ws/providers/sciotte/takeover**', (ws) => {
    ws.send(JSON.stringify({ type: 'status', state: 'ready', width: 320, height: 568 }));
    if (outcome === 'connected') {
      ws.send(JSON.stringify({ type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }));
    }
    void ws.close();
  });
  await page.goto(`/sign-in-check#token=${encodeURIComponent(token)}`);
  await expect(page.getByTestId(`sign-in-check-${outcome}`)).toBeVisible();
  await captureTone(page, theme, `sign-in-check-${outcome}`);
}

/** The provider sign-in modal: TrainingPeaks' warning notice, then a refused login's error medallion. */
async function sweepSciotteModal(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'user', skipProvidersRoute: true });
  await page.route('**/api/providers', (route) =>
    route.fulfill(
      json({
        providers: [
          { provider: 'sciotte_trainingpeaks', display_name: 'TrainingPeaks', requires_oauth: false, connected: false, capabilities: ['activities'], consent_required: true },
        ],
      }),
    ),
  );
  await page.route('**/api/providers/sciotte/login', (route) =>
    route.fulfill(json({ status: 'error', error: 'TrainingPeaks did not accept that username and password.' })),
  );
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByTestId('settings-menu-connections').click();
  await page.getByTestId('provider-row-sciotte_trainingpeaks').getByRole('button', { name: 'Connect', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('note')).toBeVisible();
  await captureTone(page, theme, 'sciotte-modal-notice');
  await dialog.getByLabel('Username').fill('coach-account');
  await dialog.getByLabel('Password', { exact: true }).fill('not-a-real-password');
  await dialog.getByLabel('I understand that TrainingPeaks could suspend my account, and I accept that risk.').check();
  await dialog.getByRole('button', { name: 'Log In' }).click();
  await expect(dialog.getByText('TrainingPeaks did not accept that username and password.')).toBeVisible();
  await captureTone(page, theme, 'sciotte-modal-error');
}

/** A success toast, from copying a reply in the open thread. */
async function sweepToast(page: Page, theme: (typeof THEMES)[number]) {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  await setupDashboardMocks(page, { role: 'user' });
  await page.route('**/api/chat/conversations/*/messages**', (route) => route.fulfill(json(THREAD_MESSAGES)));
  await page.route('**/api/chat/conversations**', async (route, request) => {
    const url = request.url();
    if (url.includes('/messages') || url.includes('/read')) {
      await route.fallback();
      return;
    }
    await route.fulfill(json(THREAD_CONVERSATIONS));
  });
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await page.goto('/#chat/conv-sweep');
  await expect(page.getByTestId('message-row').first()).toBeVisible();
  await setTheme(page, theme);
  const lastReply = page.getByTestId('message-row').last();
  await lastReply.hover();
  await lastReply.getByTitle('Copy message').click();
  await expect(page.getByText('Copied', { exact: false }).first()).toBeVisible();
  // The toast slides in (DESIGN.md §7); a capture inside it reads as faded ink.
  await page.waitForTimeout(600);
  await page.screenshot({ path: `design-sweep/${theme}/tones-toast-success.png` });
}

test.describe('design sweep — tone surfaces', () => {
  test.describe.configure({ timeout: 120_000 });
  for (const theme of THEMES) {
    test(`the sign-up error renders in ${theme}`, async ({ page }) => sweepRegisterError(page, theme));
    test(`Platform Settings' saved state renders in ${theme}`, async ({ page }) => sweepPlatformSettings(page, theme));
    test(`the globally-disabled tools banner renders in ${theme}`, async ({ page }) => sweepToolBanner(page, theme));
    test(`pending users' actions render in ${theme}`, async ({ page }) => sweepPendingUsers(page, theme));
    test(`billing's payment problem renders in ${theme}`, async ({ page }) => sweepBillingProblem(page, theme));
    test(`the sign-in check's connected state renders in ${theme}`, async ({ page }) => sweepSignInCheck(page, theme, 'connected'));
    test(`the sign-in check's closed state renders in ${theme}`, async ({ page }) => sweepSignInCheck(page, theme, 'closed'));
    test(`the provider sign-in modal renders in ${theme}`, async ({ page }) => sweepSciotteModal(page, theme));
    test(`a success toast renders in ${theme}`, async ({ page }) => sweepToast(page, theme));
  }
});
