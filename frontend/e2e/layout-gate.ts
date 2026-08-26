// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Measures a rendered surface against the gutter its layout table declares
// ABOUTME: Geometry, not pixel diffing — no baselines to churn, so it fails on defects not redesigns

import { type Page } from '@playwright/test';
import { MD_BREAKPOINT_PX, PAGE_GUTTER_PX } from '../src/constants/surfaceLayout';

/**
 * Why this measures instead of screenshotting.
 *
 * `design-sweep.visual.spec.ts` already rendered every surface and captured a
 * full-page PNG of each. Its only assertion was that at least one surface was
 * reachable, so it photographed a Groups page welded to the viewport edge on
 * every run and reported green for a year. Screenshots address a reviewer;
 * nobody opens the artifact.
 *
 * A pixel-diff baseline would fail instead — and would also fail on every
 * intentional restyle, which is how visual suites get muted. Geometry is the
 * middle: it encodes the one contract that is never deliberately broken —
 * content does not collide with the edge of its pane — and says nothing about
 * colour, type or spacing scale.
 */

/** One element sitting closer to a pane edge than its surface allows. */
export interface EdgeCollision {
  edge: 'left' | 'right' | 'top';
  /** Distance from the pane edge, px. Negative means the element overhangs it. */
  actualPx: number;
  /** Distance the surface's layout requires, px. */
  expectedPx: number;
  /** Enough of the element to find it in the source. */
  element: string;
}

export interface LayoutReport {
  /** False when no `[data-page-shell]` was on screen — the surface never mounted. */
  shellFound: boolean;
  /** What the shell declared: `padded` or `bleed`. */
  layout: string | null;
  /** Gutter required at the viewport this ran at. */
  gutterPx: number;
  collisions: EdgeCollision[];
  /** Horizontal overflow of the document, px. Any value > 0 is a defect. */
  documentOverflowPx: number;
}

/**
 * Measure the surface currently on screen.
 *
 * `padded` shells must keep every direct child a full gutter clear of the pane
 * on the left, right and top. `bleed` shells own their gutters internally, so
 * only the hard constraint applies there: nothing may overhang the pane.
 */
export async function measurePageLayout(page: Page): Promise<LayoutReport> {
  return page.evaluate(
    ({ mdBreakpoint, base, md }) => {
      const shell = document.querySelector('[data-page-shell]');
      const documentOverflowPx = Math.max(
        0,
        document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      const gutterPx = window.innerWidth >= mdBreakpoint ? md : base;

      if (!shell) {
        return { shellFound: false, layout: null, gutterPx, collisions: [], documentOverflowPx };
      }

      const layout = shell.getAttribute('data-page-layout');
      const paneBox = shell.getBoundingClientRect();
      // A scrolled pane legitimately puts its first child above the top gutter.
      const atScrollTop = shell.scrollTop <= 1;
      const collisions: EdgeCollision[] = [];

      for (const child of Array.from(shell.children)) {
        const box = child.getBoundingClientRect();
        if (box.width === 0 || box.height === 0) continue;
        // Fixed children (toasts, dialogs) are positioned against the viewport,
        // not the pane, so the pane's gutter says nothing about them.
        if (getComputedStyle(child).position === 'fixed') continue;

        const classes = (child.getAttribute('class') ?? '').split(/\s+/).filter(Boolean);
        const element = `<${child.tagName.toLowerCase()} class="${classes.slice(0, 4).join(' ')}">`;

        // Overhanging the pane is a defect on every surface, padded or bleed.
        if (box.right > paneBox.right + 1) {
          collisions.push({
            edge: 'right',
            actualPx: Math.round(paneBox.right - box.right),
            expectedPx: layout === 'padded' ? gutterPx : 0,
            element,
          });
        } else if (layout === 'padded' && box.right > paneBox.right - gutterPx + 1) {
          collisions.push({
            edge: 'right',
            actualPx: Math.round(paneBox.right - box.right),
            expectedPx: gutterPx,
            element,
          });
        }

        if (layout !== 'padded') continue;

        if (box.left < paneBox.left + gutterPx - 1) {
          collisions.push({
            edge: 'left',
            actualPx: Math.round(box.left - paneBox.left),
            expectedPx: gutterPx,
            element,
          });
        }
        if (atScrollTop && box.top < paneBox.top + gutterPx - 1) {
          collisions.push({
            edge: 'top',
            actualPx: Math.round(box.top - paneBox.top),
            expectedPx: gutterPx,
            element,
          });
        }
      }

      return { shellFound: true, layout, gutterPx, collisions, documentOverflowPx };
    },
    { mdBreakpoint: MD_BREAKPOINT_PX, base: PAGE_GUTTER_PX.base, md: PAGE_GUTTER_PX.md },
  );
}

/**
 * Render a report as failure lines, empty when the surface is clean.
 *
 * Returned rather than asserted so a sweep can walk every surface and report
 * all of them at once. A gate that stops at the first offender turns a
 * one-pass fix into one push per page.
 */
export function describeLayoutFailures(surface: string, report: LayoutReport): string[] {
  if (!report.shellFound) return [`${surface}: no [data-page-shell] on screen — surface never mounted`];

  const lines = report.collisions.map(
    (c) =>
      `${surface} [${report.layout}]: ${c.element} sits ${c.actualPx}px from the ${c.edge} pane edge, needs ${c.expectedPx}px`,
  );
  if (report.documentOverflowPx > 0) {
    lines.push(`${surface}: document scrolls horizontally by ${report.documentOverflowPx}px`);
  }
  return lines;
}
