// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Finds text painted over other text, or out of its own cell, from rendered glyph boxes
// ABOUTME: Element boxes look fine when a word escapes its grid track, so this measures the glyphs themselves

import { expect, type Page } from '@playwright/test';

interface TextScan {
  textRuns: number;
  collisions: string[];
  spills: string[];
}

/** Runs in the page, so it can only use what the browser gives it. Read-only. */
function scanText(root: string | null): TextScan {
  const scope = root === null ? document.body : document.querySelector(root);
  if (scope === null) return { textRuns: 0, collisions: [], spills: [] };

  // Clip against every clipping ancestor: what they hide is not painted.
  const painted = (el: Element, rect: DOMRect) => {
    const box = { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
    for (let cur: Element | null = el; cur && cur !== document.documentElement; cur = cur.parentElement) {
      const style = getComputedStyle(cur);
      const bounds = cur.getBoundingClientRect();
      if (style.overflowX !== 'visible') {
        box.left = Math.max(box.left, bounds.left);
        box.right = Math.min(box.right, bounds.right);
      }
      if (style.overflowY !== 'visible') {
        box.top = Math.max(box.top, bounds.top);
        box.bottom = Math.min(box.bottom, bounds.bottom);
      }
    }
    return box.right - box.left > 1 && box.bottom - box.top > 1 ? box : null;
  };

  // A modal over the page is layering, not scrambling: its scrim hides what is underneath.
  const layer = (el: Element) => {
    let z = '';
    for (let cur: Element | null = el; cur; cur = cur.parentElement) {
      const style = getComputedStyle(cur);
      if (style.position === 'fixed' || (style.position === 'absolute' && style.zIndex !== 'auto')) z += `${style.zIndex}/`;
    }
    return z;
  };

  const runs: Array<{ el: Element; text: string; box: NonNullable<ReturnType<typeof painted>> }> = [];
  const spills = new Set<string>();
  const walker = document.createTreeWalker(scope, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = (node.textContent ?? '').trim();
    const el = node.parentElement;
    if (!text || !el) continue;
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) continue;
    const bounds = el.getBoundingClientRect();
    if (bounds.width === 0 || bounds.height === 0) continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    const cell = el.closest('[role="cell"], [role="rowheader"]');
    for (const rect of Array.from(range.getClientRects())) {
      if (rect.width < 2 || rect.height < 2) continue;
      const box = painted(el, rect);
      if (box === null) continue;
      runs.push({ el, text, box });
      if (cell !== null) {
        const by = box.right - cell.getBoundingClientRect().right;
        if (by > 1) spills.add(`"${text.slice(0, 60)}" leaves its cell by ${Math.round(by)}px`);
      }
    }
  }

  const collisions = new Set<string>();
  for (let i = 0; i < runs.length; i += 1) {
    for (let j = i + 1; j < runs.length; j += 1) {
      const a = runs[i];
      const b = runs[j];
      if (a.el === b.el) continue;
      const wide = Math.min(a.box.right, b.box.right) - Math.max(a.box.left, b.box.left);
      const tall = Math.min(a.box.bottom, b.box.bottom) - Math.max(a.box.top, b.box.top);
      // Over a pixel wide and most of a line tall: tight leading between two lines is not a collision.
      if (wide <= 1 || tall < 0.4 * Math.min(a.box.bottom - a.box.top, b.box.bottom - b.box.top)) continue;
      if (layer(a.el) !== layer(b.el)) continue;
      collisions.add(`"${a.text.slice(0, 60)}" over "${b.text.slice(0, 60)}" by ${Math.round(wide)}px`);
    }
  }
  return { textRuns: runs.length, collisions: [...collisions], spills: [...spills] };
}

/**
 * Fails on text over text inside `root`, at each sideways position of its
 * scroller: a table wider than the window hides its right half from one reading.
 * `minRuns` stops an empty page, which has nothing to collide, from passing.
 */
export async function expectNoCollidingText(page: Page, label: string, root: string | null, minRuns: number, scroller?: string) {
  const stops = scroller === undefined ? [0] : [0, 1];
  let measured = 0;
  const found: string[] = [];
  for (const stop of stops) {
    if (scroller !== undefined) {
      await page.locator(scroller).evaluate((el, fraction) => {
        el.scrollLeft = (el.scrollWidth - el.clientWidth) * fraction;
      }, stop);
    }
    const scan = await page.evaluate(scanText, root);
    measured = Math.max(measured, scan.textRuns);
    found.push(...scan.collisions, ...scan.spills);
  }
  expect(measured, `${label}: measured too little text to mean anything`).toBeGreaterThanOrEqual(minRuns);
  expect([...new Set(found)], `${label}: text painted over text`).toEqual([]);
}

/** Puts the app in French, the product default. */
export async function useFrench(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem('pierre_app_language', 'fr');
  });
}
