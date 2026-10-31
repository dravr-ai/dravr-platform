// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders every top-level surface in both themes for design review
// ABOUTME: Machine gates prove token mechanics; only a rendered screen proves it looks right

import { test, expect, type Page } from '@playwright/test';
import { webNavLabels } from '@pierre/shared-constants';
import {
  setupDashboardMocks,
  loginToDashboard,
  signInThroughHostedPage,
  mockHomeFirstArrival,
  navigateToTab,
  ONBOARDED_STATUS,
} from './test-helpers';
import { describeLayoutFailures, measurePageLayout } from './layout-gate';
import { mockCoach } from './coach-home-mocks';

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

// Home's first arrival (carnet#1098): Getting started open on Meet your team
// and a team of three at the top of Today — the state no registry walk
// reaches, since the shared mocks model an established athlete. Wide, Today
// is the docked panel; narrow, the Today line carries the count and its sheet
// opens on the same two sections. Both pass the page's layout contract.
async function sweepHomeFirstArrival(page: Page, theme: (typeof THEMES)[number]) {
  await setupDashboardMocks(page, { role: 'user', displayName: 'Camille' });
  await mockHomeFirstArrival(page);
  await loginToDashboard(page);
  page.setDefaultTimeout(5_000);
  await expect(page.getByTestId('home-getting-started')).toBeVisible();
  await expect(page.getByTestId('home-team')).toBeVisible();
  await page.waitForTimeout(700);
  await setTheme(page, theme);
  await page.screenshot({ path: `design-sweep/${theme}/user-home-first-arrival.png`, fullPage: true });
  const wide = describeLayoutFailures('user/home-first-arrival', await measurePageLayout(page));
  expect(wide, `layout contract broken:\n${wide.join('\n')}`).toEqual([]);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByTestId('home-today-peek-progress')).toHaveText('1 of 5');
  await page.getByTestId('home-today-peek').click();
  await expect(page.getByRole('dialog', { name: 'Today' }).getByTestId('home-getting-started')).toBeVisible();
  await page.waitForTimeout(700);
  await page.screenshot({ path: `design-sweep/${theme}/user-home-first-arrival-phone.png`, fullPage: true });
  const sideways = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(sideways, 'the Today sheet scrolls sideways at 390px').toBeLessThanOrEqual(0);
}

test.describe('design sweep — Home first arrival', () => {
  test.describe.configure({ timeout: 180_000 });
  for (const theme of THEMES) {
    test(`Home's first arrival renders in ${theme}`, async ({ page }) => {
      await sweepHomeFirstArrival(page, theme);
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
