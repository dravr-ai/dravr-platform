// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Walks every admin tab in both themes and fails on a surface that belongs to the other theme
// ABOUTME: Reads computed colours and chart pixels, because a class scan cannot see what a screen paints

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, mockAdminUserDrawer } from './test-helpers';

// The hash of every tab the admin rail offers. A table's body only exists once
// it has rows, so the tabs that paint one get rows below: an empty state has no
// table body to measure.
const ADMIN_TABS = [
  'users',
  'agents',
  'agent-store',
  'configuration',
  'user-tools',
  'prompts',
  'platform-settings',
  'claim-verdicts',
  'harness-config',
  'memory-worker',
  'myth-busting',
  'agent-grading',
  'activity',
  'engagement',
  'audit',
  'connections',
  'analytics',
  'billing',
  'impersonation-log',
] as const;

// Reached from the gear or a typed link rather than the rail, so the parity check
// below cannot see them and they are walked on their own.
const OFF_RAIL_TABS = ['settings', 'notifications'] as const;

const THEMES = ['light', 'dark'] as const;
type Theme = (typeof THEMES)[number];

const NOW = '2026-11-01T12:00:00Z';

const json = (body: unknown) => ({
  status: 200,
  contentType: 'application/json',
  body: JSON.stringify(body),
});

const VERDICT = {
  id: 'v1',
  tenant_id: 'tenant-a',
  user_id: 'user-1',
  agent_id: 'coach-1',
  conversation_id: 'conv-1',
  message_id: null,
  claim_text: 'Take 5 g of creatine per day for high-intensity performance.',
  category: 'supplement',
  status: 'unsupported',
  evidence_strength: 'weak',
  confidence: 0.8,
  layer_fired: 'evidence',
  explanation: 'No source backs this dose.',
  evidence_refs: 'issn:2017-creatine',
  created_at: NOW,
  disposition: null,
  disposition_reason: null,
  disposition_note: null,
  disposed_by: null,
  disposed_at: null,
};

const CONSUMPTION = {
  summary: { total_tokens: 90_000, total_calls: 120, estimated_cost_usd: 3.2 },
  breakdown: [
    { provider: 'gemini', call_type: 'chat', total_tokens: 50_000, calls: 80, cost_usd: 2 },
    { provider: 'copilot', call_type: 'tool', total_tokens: 30_000, calls: 30, cost_usd: 1 },
    { provider: 'claude', call_type: 'memory', total_tokens: 10_000, calls: 10, cost_usd: 0.2 },
  ],
  daily_series: [
    { date: '2026-10-30', tokens: 20_000, cost_usd: 0.6 },
    { date: '2026-10-31', tokens: 30_000, cost_usd: 1 },
    { date: '2026-11-01', tokens: 40_000, cost_usd: 1.6 },
  ],
};

const USAGE_ANALYTICS = {
  time_series: [
    { date: '2026-10-30', request_count: 10, error_count: 1 },
    { date: '2026-10-31', request_count: 18, error_count: 2 },
    { date: '2026-11-01', request_count: 12, error_count: 0 },
  ],
  error_rate: 2,
  average_response_time: 110,
};

async function mockRows(page: Page) {
  // E2E Vite answers an unmocked path with index.html and a 200, which reaches a
  // tab as a string and crashes it; the tabs below paint their rows from these.
  await page.route('**/api/admin/myth-busting/summary**', (route) =>
    route.fulfill(
      json({
        scope: { kind: 'tenant', tenant_id: 'tenant-a' },
        verdicts_scanned: 40,
        window_start: NOW,
        window_end: NOW,
        flagged_total: 2,
        distinct_claims: 1,
        recent_claims: [
          {
            claim_excerpt: 'Stretching prevents all injuries.',
            status: 'contradicted',
            layer_fired: 'judge',
            category: 'recovery',
            occurrences: 2,
            agent_count: 1,
            first_seen_at: NOW,
            last_seen_at: NOW,
          },
        ],
        top_agents: [
          { agent_id: 'coach-1', agent_name: 'Recovery agent', unsupported_total: 2, categories: ['recovery'] },
        ],
        top_categories: [{ category: 'recovery', flagged_total: 2, agent_count: 1 }],
      }),
    ),
  );
  await page.route('**/api/admin/audit-events**', (route) =>
    route.fulfill(
      json({
        success: true,
        message: '2 audit events',
        data: {
          events: [
            {
              id: 'a1',
              occurred_at: NOW,
              action: 'pre_approval.remove',
              summary: 'Removed the pre-approval of invited@example.com (no account deleted)',
              outcome: 'success',
              source: 'console',
              actor_user_id: 'admin-1',
              actor_email: 'admin@test.com',
              tenant_id: 'tenant-a',
              metadata: { method: 'DELETE', route: '/api/admin/pre-approved-emails/{email}', status: 200, params: { email: 'invited@example.com' } },
            },
            {
              id: 'a2',
              occurred_at: NOW,
              action: 'settings.signup.update',
              summary: 'Sign-up settings: daily limit 100 → 1000',
              outcome: 'refused',
              source: 'cli',
              actor_user_id: null,
              actor_email: null,
              tenant_id: null,
              metadata: { method: 'PUT', route: '/admin/settings/signup', status: 403, before: { daily_limit: 100 }, after: { daily_limit: 1000 } },
            },
          ],
          next_before: null,
          next_before_id: null,
        },
      }),
    ),
  );
  await page.route('**/api/admin/impersonate/sessions', (route) =>
    route.fulfill(
      json({
        sessions: [
          {
            id: 's1',
            impersonator_id: 'admin-1',
            impersonator_email: 'admin@test.com',
            target_user_id: 'user-1',
            target_user_email: 'athlete@test.com',
            reason: 'Support ticket',
            started_at: NOW,
            ended_at: NOW,
            is_active: false,
            duration_seconds: 120,
          },
        ],
        total_count: 1,
        next_before: null,
        next_before_id: null,
      }),
    ),
  );
  await page.route('**/api/admin/claim-verdicts/health**', (route) =>
    route.fulfill(
      json({
        window_start: NOW,
        window_days: 30,
        totals: { flagged: 6, disposed: 4, true_catches: 1, false_positives: 3, unsure: 0 },
        false_positive_rate: 0.75,
        by_layer: [],
        by_category: [],
        by_agent: [],
        by_reason: [],
      }),
    ),
  );
  await page.route('**/api/admin/claim-verdicts?**', (route) =>
    route.fulfill(
      json({
        verdicts: [VERDICT],
        total: 1,
        status_counts: { supported: 0, unsupported: 1, contradicted: 0, rhetorical: 0, unverifiable: 0 },
      }),
    ),
  );
  await page.route('**/api/admin/agent-grading/summary**', (route) =>
    route.fulfill(
      json({
        scope: { kind: 'tenant', tenant_id: 'tenant-a' },
        verdicts_scanned: 40,
        judge_era_after: '2026-08-11',
        verdicts_before_judge_era: 0,
        unattributed_verdicts: 0,
        grades: [
          {
            agent_id: 'coach-1',
            agent_name: 'Recovery agent',
            grade: 'B',
            score: 0.81,
            total_verdicts: 40,
            supported: 30,
            unsupported: 8,
            unsupported_prescription: 0,
            contradicted: 2,
            rhetorical: 0,
            unverifiable: 0,
            false_positives: 0,
          },
        ],
      }),
    ),
  );
  await page.route('**/api/admin/evals/verdict-stats**', (route) =>
    route.fulfill(
      json({
        window_start: NOW,
        window_days: 30,
        totals: { supported: 8, unsupported: 2, contradicted: 1, rhetorical: 0, unverifiable: 1 },
        daily: [],
      }),
    ),
  );
  await page.route('**/admin/usage/llm-consumption**', (route) => route.fulfill(json(CONSUMPTION)));
  await page.route('**/api/dashboard/analytics**', (route) =>
    route.fulfill(json(USAGE_ANALYTICS)),
  );
  await page.route('**/api/admin/agents**', (route) =>
    route.fulfill(
      json({
        agents: [
          {
            id: 'coach-1',
            title: 'Marathon Agent',
            category: 'running',
            token_count: 1200,
            use_count: 12,
            tags: [],
            system_prompt: 'Plan the marathon.',
            created_at: NOW,
            updated_at: NOW,
          },
        ],
        total: 1,
        metadata: { timestamp: NOW, api_version: '1' },
      }),
    ),
  );
  await page.route('**/api/admin/engagement**', (route) =>
    route.fulfill(
      json({
        scope: { kind: 'all_tenants' },
        users: { total: 3, daily: 1, weekly: 2, monthly: 2, dormant: 1 },
        total_agents: 1,
        published_agents: 1,
        leaderboard: [{ id: 'coach-1', title: 'Marathon Agent', category: 'running', use_count: 12 }],
      }),
    ),
  );
}

interface ThemeViolation {
  kind: string;
  detail: string;
}

/**
 * Runs in the page. A surface "belongs to the other theme" when it paints a large
 * light block in dark mode or a large dark block in light mode; text that cannot
 * be read against what is really behind it is the same bug seen from the ink.
 * Tokens that legitimately flip (a filled primary button) are read from the
 * stylesheet, so nothing here names a colour of the palette.
 */
function scanForMix(theme: Theme): ThemeViolation[] {
  type Rgba = [number, number, number, number];
  const parse = (value: string): Rgba | null => {
    const m = value.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const parts = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
    if (parts.length < 3 || parts.some(Number.isNaN)) return null;
    return [parts[0], parts[1], parts[2], parts.length > 3 ? parts[3] : 1];
  };
  const over = (top: Rgba, under: Rgba): Rgba => {
    const a = top[3];
    return [
      top[0] * a + under[0] * (1 - a),
      top[1] * a + under[1] * (1 - a),
      top[2] * a + under[2] * (1 - a),
      1,
    ];
  };
  const luminance = ([r, g, b]: Rgba) => {
    const f = (c: number) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const contrast = (a: Rgba, b: Rgba) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  const rootStyle = getComputedStyle(document.documentElement);
  const token = (name: string): Rgba | null => {
    const raw = rootStyle.getPropertyValue(name).trim();
    return raw ? parse(`rgb(${raw})`) : null;
  };
  const paper = token('--color-surface') ?? [0, 0, 0, 1];
  const backdrop = (el: Element | null): Rgba => {
    const chain: Rgba[] = [];
    for (let cur = el; cur; cur = cur.parentElement) {
      const bg = parse(getComputedStyle(cur).backgroundColor);
      if (bg && bg[3] > 0) {
        chain.push(bg);
        if (bg[3] >= 1) break;
      }
    }
    return chain.reduceRight<Rgba>((under, top) => over(top, under), paper);
  };
  const filled = [
    '--color-primary',
    '--color-primary-container',
    '--color-on-surface',
    '--color-error',
    '--color-warning',
    '--color-success',
    '--color-info',
  ]
    .map(token)
    .filter((c): c is Rgba => c !== null);
  const isFilledToken = (bg: Rgba) =>
    filled.some((c) => c[0] === bg[0] && c[1] === bg[1] && c[2] === bg[2]);
  const describe = (el: Element) => {
    const cls = typeof el.className === 'string' ? el.className.trim().slice(0, 70) : '';
    const text = (el.textContent ?? '').trim().slice(0, 40);
    return `<${el.tagName.toLowerCase()} class="${cls}"> "${text}"`;
  };

  const out: ThemeViolation[] = [];
  const seen = new Set<string>();
  const report = (kind: string, detail: string) => {
    const key = `${kind}:${detail}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push({ kind, detail });
    }
  };

  for (const el of Array.from(document.querySelectorAll('body *'))) {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') continue;
    const box = el.getBoundingClientRect();
    if (box.width === 0 || box.height === 0) continue;

    const bg = parse(style.backgroundColor);
    if (bg && bg[3] >= 0.9 && box.width >= 48 && box.height >= 20 && !isFilledToken(bg)) {
      const lum = luminance(bg);
      if (theme === 'dark' && lum > 0.5) report('light surface in dark theme', describe(el));
      if (theme === 'light' && lum < 0.05 && box.height >= 40) {
        report('dark surface in light theme', describe(el));
      }
    }

    const hasOwnText = Array.from(el.childNodes).some(
      (n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? '').trim() !== '',
    );
    if (hasOwnText && Number(style.opacity) > 0.5 && !(el as HTMLButtonElement).disabled) {
      const ink = parse(style.color);
      if (ink) {
        const under = backdrop(el);
        const ratio = contrast(over(ink, under), under);
        if (ratio < 2) report(`unreadable text (${ratio.toFixed(2)}:1)`, describe(el));
      }
    }

    // A white or black hairline only exists in one theme.
    for (const side of ['Top', 'Right', 'Bottom', 'Left'] as const) {
      const edge = parse(style[`border${side}Color`]);
      if (!edge || edge[3] === 0 || parseFloat(style[`border${side}Width`]) === 0) continue;
      const white = edge[0] >= 250 && edge[1] >= 250 && edge[2] >= 250;
      const black = edge[0] <= 5 && edge[1] <= 5 && edge[2] <= 5;
      if (theme === 'light' && white) report('white hairline in light theme', describe(el));
      if (theme === 'dark' && black) report('black hairline in dark theme', describe(el));
      // A divider left on Tailwind's default border colour (gray-200) stays light in
      // every theme: a bright rule across a dark list, whatever its exact shade.
      if (theme === 'dark' && edge[3] >= 0.9 && luminance(edge) > 0.5 && !isFilledToken(edge)) {
        report('bright hairline in dark theme', describe(el));
      }
    }
  }

  // Chart.js paints a bitmap: nothing above can see it. A series drawn in an ink
  // meant for the other theme leaves most of the opaque pixels matching the card.
  // 1.3 sits under the grid lines, which are faint on purpose (about 1.4:1 in light).
  for (const canvas of Array.from(document.querySelectorAll('canvas'))) {
    if (canvas.width === 0 || canvas.height === 0) continue;
    const ctx = canvas.getContext('2d');
    if (!ctx) continue;
    const under = backdrop(canvas);
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
    let painted = 0;
    let lost = 0;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i + 3] < 200) continue;
      painted += 1;
      if (contrast([data[i], data[i + 1], data[i + 2], 1], under) < 1.3) lost += 1;
    }
    if (painted > 0 && lost / painted > 0.15) {
      report(
        'chart series lost against its card',
        `${Math.round((100 * lost) / painted)}% of painted pixels, canvas "${canvas.getAttribute('aria-label') ?? ''}"`,
      );
    }
  }
  return out;
}

const CRASH_TEXT = 'Something went wrong';

/** One user with an override, for the two surfaces that need a user picked first. */
async function mockPickedUser(page: Page) {
  await mockAdminUserDrawer(page, NOW);
  const tool = (tool_name: string, source: string) => ({
    tool_name,
    display_name: tool_name,
    description: 'A tool',
    category: 'fitness',
    is_enabled: true,
    source,
    min_plan: 'starter',
  });
  await page.route(/\/api\/admin\/tools\/user\/[^/]+$/, (route) =>
    route.fulfill(
      json({
        success: true,
        message: 'ok',
        data: [tool('get_activities', 'user_override'), tool('analyze_activity', 'default'), tool('get_sleep', 'default')],
      }),
    ),
  );
}

async function openTab(page: Page, tab: string) {
  await page.evaluate((hash) => {
    window.location.hash = hash;
  }, tab);
  // Entrance animation plus the colour transitions the variables re-trigger;
  // sampling inside them reads interpolated colours as a bug.
  await page.waitForTimeout(1200);
}

async function scanTab(page: Page, theme: Theme, tab: string, found: string[]) {
  await openTab(page, tab);
  const crashed = page.getByText(CRASH_TEXT);
  if (await crashed.count()) {
    await page.getByText('Technical Details').first().click();
    found.push(`${theme}/${tab} — tab crashed: ${(await page.locator('details').first().innerText()).slice(0, 300)}`);
    await page.screenshot({ path: `test-results/admin-theme-mix/${theme}/${tab}.png`, fullPage: true });
    // The boundary sits above the tabs and holds its error across a hash change:
    // without a new session every later tab would scan the error card and pass.
    await loginToDashboard(page);
    return;
  }
  // The sign-in page paints no mix at all, so a lost session would read as a
  // clean sweep. Only the admin shell counts as having looked at this tab.
  await expect(page.locator('main'), `#${tab} is not the app shell`).toBeVisible();
  // The tab itself or a section of it (`#settings/profile`), never a neighbour
  // that merely contains its name (`#platform-settings`).
  const hash = new URL(page.url()).hash;
  expect(hash === `#${tab}` || hash.startsWith(`#${tab}/`), `#${tab} was redirected to ${hash}`).toBe(true);
  // Every operator screen opens with its theory panel (carnet#1441), whose body
  // is mounted only while open: open it, so the scan measures that body in this
  // theme and not only the one-line summary.
  const theory = page.getByTestId('screen-theory');
  if (await theory.count()) {
    await theory.locator('summary').click();
    await expect(theory.getByText('How the numbers are computed')).toBeVisible();
  }
  const violations = await page.evaluate(scanForMix, theme);
  await page.screenshot({ path: `test-results/admin-theme-mix/${theme}/${tab}.png`, fullPage: true });
  for (const v of violations) found.push(`${theme}/${tab} — ${v.kind}: ${v.detail}`);
}

for (const theme of THEMES) {
  test(`every admin tab paints only the ${theme} theme`, async ({ page }) => {
    test.setTimeout(180_000);
    // The real path: a stored preference, so useTheme resolves the scheme and a
    // chart re-reads its tokens. Toggling `.dark` by hand would leave the hook
    // believing the old scheme and prove nothing about the charts.
    await page.addInitScript((t) => {
      window.localStorage.setItem('dravr.theme', t);
    }, theme);
    // The content pane scrolls inside the shell, so a full-page capture stops at
    // the fold: tall enough to look at what the scan measured.
    await page.setViewportSize({ width: 1440, height: 2200 });
    await setupDashboardMocks(page, { role: 'super_admin' });
    await mockRows(page);
    await loginToDashboard(page);

    const found: string[] = [];
    for (const tab of [...ADMIN_TABS, ...OFF_RAIL_TABS]) {
      await scanTab(page, theme, tab, found);
    }
    // Overlays only exist once opened, so the walk above never measures them.
    await openTab(page, 'claim-verdicts');
    await page.getByText('Take 5 g of creatine').first().click();
    await page.waitForTimeout(600);
    for (const v of await page.evaluate(scanForMix, theme)) {
      found.push(`${theme}/claim-verdicts drawer — ${v.kind}: ${v.detail}`);
    }
    await page.screenshot({ path: `test-results/admin-theme-mix/${theme}/claim-verdicts-drawer.png` });

    // The tool list and the user drawer only exist once a user is picked.
    await mockPickedUser(page);
    await openTab(page, 'user-tools');
    await page.getByRole('button', { name: /alice@acme.com/ }).click();
    await expect(page.getByText('User Override').first()).toBeVisible();
    await page.waitForTimeout(600);
    for (const v of await page.evaluate(scanForMix, theme)) {
      found.push(`${theme}/user-tools list — ${v.kind}: ${v.detail}`);
    }
    await page.screenshot({ path: `test-results/admin-theme-mix/${theme}/user-tools-list.png` });

    await openTab(page, 'users');
    await page.getByRole('button', { name: /^Active/ }).first().click();
    await page.getByText('alice@acme.com').first().click();
    await expect(page.getByText('Usage (Month-to-Date)')).toBeVisible();
    await page.waitForTimeout(600);
    for (const v of await page.evaluate(scanForMix, theme)) {
      found.push(`${theme}/user drawer — ${v.kind}: ${v.detail}`);
    }
    await page.screenshot({ path: `test-results/admin-theme-mix/${theme}/user-drawer.png` });

    expect(
      await page.evaluate(() => document.documentElement.classList.contains('dark')),
    ).toBe(theme === 'dark');
    // Every offender at once: a gate that stops at the first turns one sweep
    // into one push per tab.
    expect(found, `${found.length} theme mix(es):\n${found.join('\n')}`).toEqual([]);
  });
}

for (const theme of THEMES) {
  test(`a bare form control follows the ${theme} theme in every state`, async ({ page }) => {
    await page.addInitScript((t) => {
      window.localStorage.setItem('dravr.theme', t);
    }, theme);
    await setupDashboardMocks(page, { role: 'super_admin' });
    await loginToDashboard(page);
    await expect(page.locator('main')).toBeVisible();
    const painted = await page.evaluate(() => {
      const read = (props: Partial<HTMLInputElement>) => {
        const input = document.createElement('input');
        Object.assign(input, props);
        document.body.appendChild(input);
        const { backgroundColor } = getComputedStyle(input);
        input.remove();
        return backgroundColor;
      };
      const token = (name: string) =>
        `rgb(${getComputedStyle(document.documentElement).getPropertyValue(name).trim().split(/\s+/).join(', ')})`;
      return {
        // A control with no type attribute is the one the forms plugin names on its own.
        untyped: read({}),
        text: read({ type: 'text' }),
        unchecked: read({ type: 'checkbox' }),
        checked: read({ type: 'checkbox', checked: true }),
        radio: read({ type: 'radio', checked: true }),
        ground: token('--color-surface-container-lowest'),
        fill: token('--color-primary'),
      };
    });
    expect(painted.untyped).toBe(painted.ground);
    expect(painted.text).toBe(painted.ground);
    expect(painted.unchecked).toBe(painted.ground);
    expect(painted.checked).toBe(painted.fill);
    expect(painted.radio).toBe(painted.fill);
  });
}

test('a checked control draws its mark in the ink of its fill', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('dravr.theme', 'dark');
  });
  await setupDashboardMocks(page, { role: 'super_admin' });
  await loginToDashboard(page);
  await expect(page.locator('main')).toBeVisible();
  const mark = (type: string, dark: boolean) =>
    page.evaluate(
      ([kind, isDark]) => {
        document.documentElement.classList.toggle('dark', isDark as boolean);
        const input = document.createElement('input');
        input.type = kind as string;
        input.checked = true;
        document.body.appendChild(input);
        const { backgroundImage } = getComputedStyle(input);
        input.remove();
        return backgroundImage;
      },
      [type, dark] as const,
    );
  // White on dark's pale primary is 1.7:1, and a data URI cannot read a variable,
  // so index.css spells this token out: a new value must move that literal with it.
  const onPrimary = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--color-on-primary').trim(),
  );
  const ink = onPrimary
    .split(' ')
    .map((channel) => Number(channel).toString(16).padStart(2, '0'))
    .join('');
  expect(ink, `--color-on-primary read as "${onPrimary}"`).toMatch(/^[0-9a-f]{6}$/);
  expect(await mark('checkbox', true)).toContain(ink);
  expect(await mark('radio', true)).toContain(ink);
  expect(await mark('checkbox', false)).toContain('white');
  expect(await mark('radio', false)).toContain('white');
});

test('the table rows this gate depends on actually rendered', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('dravr.theme', 'dark');
  });
  await setupDashboardMocks(page, { role: 'super_admin' });
  await mockRows(page);
  await loginToDashboard(page);
  // ADMIN_TABS is a hand list: a tab added to the rail must be added to the walk.
  await expect(page.locator('main')).toBeVisible();
  await expect(page.locator('aside nav button'), 'admin rail and ADMIN_TABS disagree').toHaveCount(ADMIN_TABS.length);

  // A renamed tab or a mock that stops matching would otherwise turn the sweep
  // above into a walk through empty states, and an empty state has no table body.
  for (const [tab, text] of [
    ['claim-verdicts', 'Take 5 g of creatine'],
    ['agent-grading', 'coach-1'],
    ['myth-busting', 'Stretching prevents all injuries.'],
    ['impersonation-log', 'athlete@test.com'],
    ['engagement', 'Marathon Agent'],
  ] as const) {
    await page.evaluate((hash) => {
      window.location.hash = hash;
    }, tab);
    await expect(page.getByText(text).first(), `#${tab} rendered no rows`).toBeVisible();
  }
  // Nothing paints a chart but the data: without these the pixel check has no
  // canvas to read and the dark-only series come back unseen.
  await page.evaluate(() => {
    window.location.hash = 'analytics';
  });
  await expect(page.locator('canvas').first()).toBeVisible();
  // The charts arrive one query at a time, so the first being up says nothing of the rest.
  // Usage Analytics draws its series, LLM Consumption its line, doughnut and bars.
  await expect.poll(() => page.locator('canvas').count()).toBeGreaterThanOrEqual(4);
});
