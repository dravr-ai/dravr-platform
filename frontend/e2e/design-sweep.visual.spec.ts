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
  'Coach Store',
  'Tool Management',
  'Platform Settings',
  'Service Tokens',
  'Harness Config',
  'Coach Notes Audit',
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

test.describe('design sweep', () => {
  // Each surface carries an entrance animation, a theme-transition settle and a
  // full-page capture. The default 30s cap is sized for a single interaction,
  // not a walk of the whole app.
  test.describe.configure({ timeout: 180_000 });

  for (const theme of THEMES) {
    test(`user surfaces render in ${theme}`, async ({ page }) => {
      await sweep(page, { role: 'user', surfaces: USER_SURFACES, theme });
    });

    test(`admin surfaces render in ${theme}`, async ({ page }) => {
      await sweep(page, { role: 'admin', surfaces: ADMIN_SURFACES, theme });
    });
  }
});
